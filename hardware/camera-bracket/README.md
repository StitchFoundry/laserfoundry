# Camera bracket (add-on 36, issue #87)

An original, free, printable bracket that clips onto the top frame of an enclosure (made for the Ortur Enclosure 2.0, named only to say what it
is meant to fit) and holds a generic 1080p UVC webcam board looking straight down at the bed. Designed from scratch: no vendor CAD files, no
other people's brackets, no logos.

**Licence:** CC BY 4.0 for the model (`camera-bracket.scad` and anything exported from it).

## Status
- `camera-bracket.scad` is written but **has not been rendered or test-printed** (OpenSCAD was not available when it was written).
  `camera-bracket.stl` and `.3mf` are not committed yet: open the file in OpenSCAD, check the preview, then export both (F6, then Export).
- **Measure these 3 numbers** on your own frame and edit the top of the `.scad` file:

  | Parameter         | What to measure                                  | Placeholder |
  |-------------------|--------------------------------------------------|-------------|
  | `frame_thickness` | Depth of the top-frame bar the clip grips        | 20 mm       |
  | `frame_height`    | Height of the bar (the part the clip hooks over) | 20 mm       |
  | `clip_gap_slack`  | Clearance so it slides on (tune after a test)    | 0.4 mm      |

  Also check `board_w`, `board_h` and `hole_spacing` against your camera board.

## Printing
- Material: PETG preferred near a laser (PLA is fine for a first fit test). 0.2 mm layers, 4 walls, 30% infill.
- Print with the clip opening facing up so the arm and plate need no supports; the plate prints flat. Raise `tilt` only with supports on.
- Hardware: two M2.5 or M3 self-tapping screws for the camera board; one optional M3 screw and nut for the clamp hole.

## Fitting
Slide the clip over the top-frame bar, tighten the optional clamp screw, screw the camera board to the plate with the lens through the
clearance hole, then plug in USB. Camera height is the frame height plus the arm; measure it and note it in the camera setup page.
Photos: _placeholder, add after the first test print._

## MakerFoundry listing (draft, Bruce publishes after a test print)
- **Title:** LaserFoundry camera bracket for top-frame enclosures (UVC webcam, straight down)
- **Price / licence:** free, CC BY 4.0. **Category:** 3D printing. **Tags:** laser, camera, bracket, enclosure, webcam, openscad, parametric
- **Description:** A clip-on, fully parametric mount that holds a cheap 1080p USB webcam above your laser bed. Edit three measurements
  in the OpenSCAD file, print in PETG, clip it on. Works with the camera setup in LaserFoundry (bed alignment and overlay).
- **Open in LaserFoundry:** link to the camera setup page (`docs/camera.md`; the in-app page is not built yet, see issue #86).
