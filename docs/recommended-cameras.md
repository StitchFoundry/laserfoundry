# Recommended cameras (issue #88, camera support #86)

LaserFoundry's camera support is still being built (see [`camera.md`](camera.md)). This page says what kind of webcam to get so it is
ready when capture lands. We do not endorse brands and there are no affiliate links.

## What to buy

A **generic USB UVC webcam** with:

- 1080p (1920 x 1080) video;
- a 120 to 160 degree lens (wide enough to see the whole bed from a short height);
- manual or fixed focus;
- USB 2.0 (plenty for 1080p);
- a cable long enough to reach the computer (about 3 m), or a USB extension cable.

## What to avoid

- Cameras that need a vendor driver or app to work: they are not UVC and will not show up in a browser or in the desktop app.
- Autofocus that hunts: the picture shifts while the laser works and the alignment drifts. Fixed focus is better.
- Very wide lenses (over 170 degrees): the bed edges lose detail and the lens correction has less to work with.

## Check that it is UVC

- **Windows:** open the Camera app. If the picture appears with no driver install, it is UVC.
- **Linux:** run `v4l2-ctl --list-devices` (package `v4l-utils`). A UVC camera is listed under "uvcvideo" or by its name with a
  `/dev/video*` entry.
- **macOS:** it appears in any camera app without a driver.

## Mounting height

Rough estimates, not measurements: the height at which a camera looking straight down sees the whole bed plus about 10 percent margin,
from `height = 1.1 x (bed width / 2) / tan(horizontal field of view / 2)`. Your lens may differ from its label, so check with a ruler
and treat the numbers as a starting point.

| Bed width | 120 degree lens | 140 degree lens | 160 degree lens |
|-----------|-----------------|-----------------|-----------------|
| 300 mm    | about 95 mm     | about 60 mm     | about 30 mm     |
| 400 mm    | about 125 mm    | about 80 mm     | about 40 mm     |
| 600 mm    | about 190 mm    | about 120 mm    | about 60 mm     |
| 900 mm    | about 285 mm    | about 180 mm    | about 85 mm     |

Wider lenses sit lower but distort more at the edges. The free printable bracket in [`hardware/camera-bracket`](../hardware/camera-bracket/README.md)
holds a webcam board looking straight down.

## Honest limits

- Accuracy depends on calibration: the lens correction and the four-point alignment against the real bed.
- Re-align after the camera is moved or bumped, and after the bed or enclosure is changed.
- The camera sees the surface it is pointed at: shiny, clear or very dark materials may be hard to see.
