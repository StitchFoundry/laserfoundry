//! The control port: an opt-in loopback JSON line protocol so a tester or agent can drive the real app.
//!
//! Off unless the app is started with `--control <port>` or `LASERFOUNDRY_CONTROL_PORT`. It listens on
//! `127.0.0.1` only and serves one client at a time. Each line is `{"id":1,"method":"...","params":{...}}`
//! and is answered by `{"id":1,"ok":true,"result":...}` or `{"id":1,"ok":false,"error":"..."}`. The
//! connection thread only moves lines; every request runs on the app's own thread in
//! [`Control::serve_pending`], once per frame, so the UI and the client never touch the session at once.
//!
//! Only engine commands and read-only inspection are reachable. Starting a job, streaming, jogging, firing
//! and opening a serial port are UI actions and are not methods here.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use laserfoundry_engine::{Session, commands};
use laserfoundry_ui_egui::LaserFoundryApp;
use serde_json::{Map, Value, json};

/// Longest accepted request line, in bytes.
const MAX_LINE_BYTES: usize = 1024 * 1024;
/// Requests a client may make in one second; more are answered with an error.
const MAX_REQUESTS_PER_SECOND: usize = 100;
/// Requests answered per frame, so a flood cannot freeze the window.
const MAX_PER_PASS: usize = 16;
/// How long a client waits for the app to answer one request.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);

/// A request line on its way to the app thread, with where its answer goes.
struct Request {
    line: String,
    reply: SyncSender<String>,
}

/// A running listener, owned by the app.
pub struct Control {
    requests: Receiver<Request>,
}

impl Control {
    /// Binds `127.0.0.1:port` and starts accepting. A failure is logged and leaves the app running without it.
    pub fn start(port: u16, ctx: &egui::Context) -> Option<Self> {
        let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
            Ok(l) => l,
            Err(e) => {
                log::error!("control port {port} could not start: {e}");
                return None;
            }
        };
        let (tx, requests) = mpsc::sync_channel(MAX_PER_PASS);
        let ctx = ctx.clone();
        let spawned = thread::Builder::new().name("laserfoundry-control".into()).spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Err(e) = client(stream, &tx, &ctx) {
                    log::debug!("control client closed: {e}");
                }
            }
        });
        if let Err(e) = spawned {
            log::error!("control port {port} could not start: {e}");
            return None;
        }
        log::warn!("control port is ON at 127.0.0.1:{port}: programs run by this user can drive the app (not the machine)");
        Some(Self { requests })
    }

    /// Answers what arrived since the last frame (at most [`MAX_PER_PASS`] requests). Call every frame from the thread that owns the app.
    pub fn serve_pending(&mut self, app: &mut LaserFoundryApp, ctx: &egui::Context) {
        for _ in 0..MAX_PER_PASS {
            let Ok(request) = self.requests.try_recv() else { break };
            let screen = ctx.content_rect().size();
            let ui = json!({
                "window": { "width": screen.x, "height": screen.y },
                "fps": ctx.input(|i| if i.unstable_dt > 0.0 { 1.0 / i.unstable_dt } else { 0.0 }),
                "state": app.inspect(),
            });
            let reply = handle_line(&mut app.session, &ui, &request.line);
            let _ = request.reply.try_send(reply.to_string());
        }
    }
}

/// Takes `--control <port>` / `--control=<port>` out of `args` and returns the port, from the flag or else
/// from `env` (the value of `LASERFOUNDRY_CONTROL_PORT`). `Ok(None)` when neither is given.
pub fn take_port(args: &mut Vec<String>, env: Option<&str>) -> Result<Option<u16>, String> {
    let mut value = env.filter(|v| !v.is_empty()).map(str::to_owned);
    let mut i = 0;
    while i < args.len() {
        let arg = args.get(i).cloned().unwrap_or_default();
        if arg == "--control" {
            args.remove(i);
            if i >= args.len() {
                return Err("--control needs a port number (1-65535)".into());
            }
            value = Some(args.remove(i));
        } else if let Some(v) = arg.strip_prefix("--control=") {
            value = Some(v.to_owned());
            args.remove(i);
        } else {
            i += 1;
        }
    }
    let Some(text) = value else { return Ok(None) };
    match text.trim().parse::<u16>() {
        Ok(port) if port >= 1 => Ok(Some(port)),
        _ => Err(format!("the control port must be a number from 1 to 65535, not \"{}\"", text.chars().take(32).collect::<String>())),
    }
}

/// Runs one request line against the session and returns the response object. Never panics on any input.
fn handle_line(session: &mut Session, ui: &Value, line: &str) -> Value {
    let request: Value = match serde_json::from_str(line.trim()) {
        Ok(v) => v,
        Err(e) => return failure(&Value::Null, &format!("not valid JSON: {e}")),
    };
    let id = match request.get("id") {
        None | Some(Value::Null) => Value::Null,
        Some(v @ (Value::Number(_) | Value::String(_))) => v.clone(),
        Some(_) => return failure(&Value::Null, "id must be a number or a string"),
    };
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return failure(&id, "missing \"method\"");
    };
    let empty = Value::Object(Map::new());
    let params = match request.get("params") {
        None | Some(Value::Null) => &empty,
        Some(p @ Value::Object(_)) => p,
        Some(_) => return failure(&id, "params must be an object"),
    };
    match method {
        "engine.execute" => {
            let Some(command) = params.get("command").and_then(Value::as_str) else {
                return failure(&id, "engine.execute needs a string \"command\"");
            };
            let inner = match params.get("params") {
                None | Some(Value::Null) => &empty,
                Some(p @ Value::Object(_)) => p,
                Some(_) => return failure(&id, "\"params\" must be an object"),
            };
            match session.execute(command, inner) {
                Ok(result) => success(&id, result),
                Err(e) => failure(&id, &e.to_string()),
            }
        }
        "engine.commands" => {
            let list: Vec<Value> =
                commands().map(|c| json!({ "id": c.id, "label": c.label, "menu": c.menu, "params": c.params_doc, "mutates": c.mutates })).collect();
            success(&id, Value::Array(list))
        }
        "ui.inspect" => success(&id, ui.clone()),
        other => failure(&id, &format!("unknown method \"{other}\" (this version has engine.execute, engine.commands, ui.inspect)")),
    }
}

fn success(id: &Value, result: Value) -> Value {
    json!({ "id": id, "ok": true, "result": result })
}

fn failure(id: &Value, error: &str) -> Value {
    json!({ "id": id, "ok": false, "error": error })
}

/// One client: reads capped lines, hands each to the app thread and writes the answer back.
fn client(stream: TcpStream, tx: &SyncSender<Request>, ctx: &egui::Context) -> io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut input = BufReader::new(stream.try_clone()?);
    let mut output = stream;
    let mut window_start = Instant::now();
    let mut in_window = 0usize;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        // Reading through `take` means a line without a newline can never grow past the cap.
        let read = (&mut input).take(MAX_LINE_BYTES as u64 + 1).read_until(b'\n', &mut buf)?;
        if read == 0 {
            return Ok(());
        }
        if buf.len() > MAX_LINE_BYTES {
            write_line(&mut output, &failure(&Value::Null, "request line too long").to_string())?;
            return Err(io::Error::other("request line too long"));
        }
        if window_start.elapsed() >= Duration::from_secs(1) {
            window_start = Instant::now();
            in_window = 0;
        }
        in_window += 1;
        if buf.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if in_window > MAX_REQUESTS_PER_SECOND {
            write_line(&mut output, &failure(&Value::Null, "too many requests; slow down").to_string())?;
            continue;
        }
        let (reply, answer) = mpsc::sync_channel(1);
        let line = String::from_utf8_lossy(&buf).into_owned();
        tx.send(Request { line, reply }).map_err(|_| io::Error::other("app closed"))?;
        ctx.request_repaint();
        let text = answer.recv_timeout(ANSWER_TIMEOUT).map_err(|_| io::Error::other("the app did not answer"))?;
        write_line(&mut output, &text)?;
    }
}

fn write_line(out: &mut impl Write, text: &str) -> io::Result<()> {
    out.write_all(text.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    mod port {
        use super::*;

        fn take(args: &[&str], env: Option<&str>) -> (Result<Option<u16>, String>, Vec<String>) {
            let mut args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
            (take_port(&mut args, env), args)
        }

        #[test]
        fn off_by_default() {
            assert_eq!(take(&["a.svg"], None), (Ok(None), vec!["a.svg".to_owned()]));
        }

        #[test]
        fn flag_and_env_with_the_flag_winning() {
            assert_eq!(take(&["--control", "7979", "a.svg"], None), (Ok(Some(7979)), vec!["a.svg".to_owned()]));
            assert_eq!(take(&["--control=8000"], None).0, Ok(Some(8000)));
            assert_eq!(take(&[], Some("7000")).0, Ok(Some(7000)));
            assert_eq!(take(&["--control", "7979"], Some("7000")).0, Ok(Some(7979)));
        }

        #[test]
        fn bad_values_are_refused() {
            for bad in ["0", "65536", "-1", "abc", "1e3", " "] {
                assert!(take(&["--control", bad], None).0.is_err(), "{bad}");
                assert!(take(&[], Some(bad)).0.is_err() || bad == " ", "{bad}");
            }
            assert!(take(&["--control"], None).0.is_err());
        }
    }

    mod protocol {
        use super::*;

        fn ask(session: &mut Session, line: &str) -> Value {
            handle_line(session, &json!({ "fps": 60 }), line)
        }

        #[test]
        fn execute_creates_a_shape_and_returns_the_result() {
            let mut s = Session::new();
            let r = ask(
                &mut s,
                r#"{"id":1,"method":"engine.execute","params":{"command":"shape.rectangle","params":{"x":0,"y":0,"width":10,"height":5}}}"#,
            );
            assert_eq!(r["ok"], true, "{r}");
            assert_eq!(r["id"], 1);
            assert_eq!(s.doc.shapes.len(), 1);
        }

        #[test]
        fn failures_are_reported_and_roll_back() {
            let mut s = Session::new();
            let r = ask(&mut s, r#"{"id":"a","method":"engine.execute","params":{"command":"no.such.command"}}"#);
            assert_eq!((r["ok"].clone(), r["id"].clone()), (json!(false), json!("a")));
            assert!(r["error"].as_str().is_some_and(|e| e.contains("no.such.command")), "{r}");
            assert!(s.doc.shapes.is_empty());
        }

        #[test]
        fn commands_and_inspect() {
            let mut s = Session::new();
            let r = ask(&mut s, r#"{"id":2,"method":"engine.commands"}"#);
            assert!(r["result"].as_array().is_some_and(|l| l.iter().any(|c| c["id"] == "shape.rectangle")));
            let r = ask(&mut s, r#"{"id":3,"method":"ui.inspect"}"#);
            assert_eq!(r["result"]["fps"], 60);
        }

        #[test]
        fn machine_methods_do_not_exist() {
            let mut s = Session::new();
            for m in ["machine.jog", "job.start", "serial.open", "job.stream"] {
                let r = ask(&mut s, &json!({ "id": 1, "method": m }).to_string());
                assert_eq!(r["ok"], false, "{m}");
            }
        }

        #[test]
        fn hostile_input_gets_an_error_not_a_panic() {
            let mut s = Session::new();
            let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
            let lines = [
                "",
                "{",
                "null",
                "[]",
                "\"x\"",
                r#"{"method":5}"#,
                r#"{"id":[1],"method":"ui.inspect"}"#,
                r#"{"id":1,"method":"engine.execute","params":[]}"#,
                r#"{"id":1,"method":"engine.execute","params":{"command":7}}"#,
                r#"{"id":1,"method":"engine.execute","params":{"command":"shape.rectangle","params":{"x":1e999,"w":"NaN"}}}"#,
                r#"{"id":1,"method":"engine.execute","params":{"command":"shape.rectangle","params":{"x":0,"y":0,"w":1e308,"h":-1e308}}}"#,
                &deep,
            ];
            for line in lines {
                let r = ask(&mut s, line);
                assert!(r.get("ok").is_some(), "{line:.40}");
            }
        }
    }

    mod socket {
        use super::*;

        /// Serves one connection with this test thread standing in for the app.
        fn connect() -> (BufReader<TcpStream>, TcpStream, Receiver<Request>) {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let (tx, requests) = mpsc::sync_channel(MAX_PER_PASS);
            thread::spawn(move || {
                if let Some(stream) = listener.incoming().flatten().next() {
                    let _ = client(stream, &tx, &egui::Context::default());
                }
            });
            let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            (BufReader::new(stream.try_clone().unwrap()), stream, requests)
        }

        #[test]
        fn a_line_is_answered_over_the_socket_on_loopback() {
            let (mut reader, mut stream, requests) = connect();
            assert!(stream.peer_addr().unwrap().ip().is_loopback());
            stream.write_all(b"{\"id\":1,\"method\":\"ui.inspect\"}\n").unwrap();
            let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
            request.reply.send("{\"id\":1,\"ok\":true}".into()).unwrap();
            let mut answer = String::new();
            reader.read_line(&mut answer).unwrap();
            assert_eq!(answer.trim(), "{\"id\":1,\"ok\":true}");
        }

        #[test]
        fn an_over_long_line_is_refused_and_the_connection_closed() {
            let (mut reader, mut stream, requests) = connect();
            let big = vec![b'x'; MAX_LINE_BYTES + 10];
            let _ = stream.write_all(&big); // the server may hang up before all of it is sent
            let mut answer = String::new();
            reader.read_line(&mut answer).unwrap();
            assert!(answer.contains("too long"), "{answer}");
            assert!(requests.try_recv().is_err());
        }
    }
}
