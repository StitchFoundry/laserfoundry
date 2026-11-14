# Camera support (work in progress, issue #86)

A cheap USB webcam above the bed will show the material under the design. This page grows as the pieces land.

**Done so far:** the maths in `crates/camera` (pure Rust, also runs in the browser build): a fisheye lens model, the 4-point
"burn and click" bed homography, per-camera profiles as JSON, the positions of the four alignment crosses, and a printable
checkerboard (SVG). Synthetic tests check placement within 0.5 mm over a 400 x 400 mm bed.

**Not done yet:** camera capture (browser `getUserMedia`, native UVC), the guided checkerboard solver for lens parameters, the
burn-and-click flow and its commands, the overlay under the canvas, and the manual test checklist (Chrome, Edge, Windows, Linux,
a real 1080p camera). Camera frames stay on the user's machine; nothing is uploaded.

What camera to buy, how to check it, and how high to mount it: [`recommended-cameras.md`](recommended-cameras.md).
