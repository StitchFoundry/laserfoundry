# Boards that reset when the serial port opens

Many ESP32 based GRBL controllers wire the USB serial chip's DTR and RTS lines to the reset (EN) and boot (GPIO0) pins. The ESP32-S3
and ESP32-C3 native USB port (no separate USB chip) reacts to the same line changes. Opening the port can then reset the board, and a
board that has just reset may come up in GRBL's Alarm state.

What LaserFoundry's browser build does (Web Serial has no open option for the starting line state):

- right after `open` it sets DTR and RTS low with `setSignals`;
- before `close` it sets them low again;
- it writes nothing to the controller for this, so no `S` word, `M3` or `M4` comes from it.

The port may still pulse once while the operating system opens it, so an Alarm after Connect is possible. Use the Unlock button in the
Machine block (it sends `$X`; the laser stays off) and check the position before a job. Re-homing is needed if the machine has homing.

| Board                                 | Reset on open                          |
|---------------------------------------|----------------------------------------|
| ESP32 with a CP210x / CH340 USB chip  | Yes, if DTR/RTS go to EN and GPIO0     |
| ESP32-S3 / C3 native USB              | Yes, DTR/RTS toggles are decoded       |
| Arduino Uno style (ATmega328P)        | Yes, DTR resets it (GRBL restarts)     |
| Boards with a native USB CDC only     | No                                     |

This table is from the public chip documentation and has not been checked against each board; real hardware has not been tested.
