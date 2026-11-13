// LaserFoundry camera bracket: an original, parametric clip-on mount that holds a generic UVC webcam board
// looking straight down. Licence: CC BY 4.0 (see README.md). Units: millimetres.
// NOT YET COMPILED OR TEST-PRINTED: render in OpenSCAD, check the preview, then export STL/3MF.

// ---- measure these 3 numbers on your frame (TODO defaults are placeholders) ----
frame_thickness = 20;   // TODO measure: depth of the top-frame bar the clip grips (mm)
frame_height    = 20;   // TODO measure: height of the bar, the part the clip hooks over (mm)
clip_gap_slack  = 0.4;  // TODO tune: extra clearance so it slides on (0.2 to 0.6)

// ---- camera board (check against your camera) ----
board_w = 32;           // board width (mm)
board_h = 32;           // board height (mm)
hole_spacing = 24;      // distance between the two mounting holes (mm)
hole_d = 2.8;           // hole diameter for M2.5 / M3 self-tapping screws
tilt = 0;               // 0 = looking straight down; positive tilts toward the machine front (degrees)

// ---- print ----
wall = 3;               // clip wall thickness (mm)
arm_len = 40;           // how far the arm reaches in from the frame (mm)
clamp_screw_d = 3.4;    // M3 clearance hole for a clamp screw (optional)
$fn = 48;

module clip() {
    inner_t = frame_thickness + clip_gap_slack;
    inner_h = frame_height + clip_gap_slack;
    difference() {
        cube([inner_t + 2 * wall, 30, inner_h + wall]);
        translate([wall, -1, -1]) cube([inner_t, 32, inner_h + 1]);
        // optional clamp screw through the outer wall
        translate([-1, 15, inner_h / 2]) rotate([0, 90, 0]) cylinder(d = clamp_screw_d, h = wall + 2);
    }
}

module arm() {
    translate([0, 0, frame_height + clip_gap_slack])
        cube([arm_len + frame_thickness + 2 * wall, 30, wall]);
}

module plate() {
    // camera plate at the end of the arm, tilted about the Y axis
    translate([arm_len + frame_thickness + 2 * wall, 15, frame_height + clip_gap_slack + wall])
    rotate([0, tilt, 0])
    difference() {
        translate([-board_w / 2, -board_h / 2, 0]) cube([board_w, board_h, wall]);
        for (s = [-1, 1]) translate([0, s * hole_spacing / 2, -1]) cylinder(d = hole_d, h = wall + 2);
        translate([0, 0, -1]) cylinder(d = 12, h = wall + 2); // lens clearance
    }
}

union() { clip(); arm(); plate(); }
