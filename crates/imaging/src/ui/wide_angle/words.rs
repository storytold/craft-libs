//! `WideAngleWords`: Adaptive Wide Angle's words.

craft_ui::words! {
    /// Adaptive Wide Angle's words, English by default (Photoshop's names). OK and Cancel are the dialog's common
    /// words.
    #[non_exhaustive]
    pub struct WideAngleWords {
        /// The title's name.
        name: "wideAngle.name" = "Adaptive Wide Angle";
        /// The accessible name of the tool rail.
        tools: "wideAngle.tools" = "Tools";
        /// The accessible name of the side column.
        settings: "wideAngle.settings" = "Settings";
        /// The Correction section's title and its model select's name.
        correction: "wideAngle.correction" = "Correction";
        /// The Auto lens model.
        auto: "wideAngle.auto" = "Auto";
        /// The Fisheye lens model.
        fisheye: "wideAngle.fisheye" = "Fisheye";
        /// The Perspective lens model.
        perspective: "wideAngle.perspective" = "Perspective";
        /// The Full Spherical lens model.
        full_spherical: "wideAngle.fullSpherical" = "Full Spherical";
        /// The scale slider's name.
        scale: "wideAngle.scale" = "Scale";
        /// The focal length slider's name.
        focal_length: "wideAngle.focalLength" = "Focal Length";
        /// The crop factor slider's name.
        crop_factor: "wideAngle.cropFactor" = "Crop Factor";
        /// The Constraints section's title; `{n}` the number of constraints.
        constraints: count "wideAngle.constraints" = "Constraints ({n})" | "Constraints ({n})";
        /// A free constraint.
        free: "wideAngle.free" = "Free";
        /// A horizontal constraint.
        horizontal: "wideAngle.horizontal" = "Horizontal";
        /// A vertical constraint.
        vertical: "wideAngle.vertical" = "Vertical";
        /// The constraint rows' Delete button and the context menu's Delete row.
        delete: "wideAngle.delete" = "Delete";
        /// The Preview check box.
        preview: "wideAngle.preview" = "Preview";
        /// The hint under the panel.
        hint: "wideAngle.hint" = "Drag on the image to add a constraint; Shift for horizontal/vertical; right-click a constraint to change or delete it.";
        /// The accessible name of the group the constraints make on the canvas, and of the constraint context menu.
        constraint_lines: "wideAngle.constraintLines" = "Constraint lines";
        /// The accessible name of a constraint row's Orientation select; `{n}` the constraint's number.
        orientation_of: template "wideAngle.orientationOf" = "Orientation of constraint {n}";
        /// The accessible name of a constraint row's Delete button; `{n}` the constraint's number.
        delete_constraint: template "wideAngle.deleteConstraint" = "Delete constraint {n}";
        /// A constraint as a keyboard part; `{n}` its number, `{orientation}` its orientation.
        constraint_part: template "wideAngle.constraintPart" = "Constraint {n}, {orientation}";
        /// The button that opens the Add Constraint form, and the form's title.
        add_constraint: "wideAngle.addConstraint" = "Add Constraint…";
        /// The Add Constraint form's start point row.
        start: "wideAngle.start" = "Start";
        /// The Add Constraint form's end point row.
        end: "wideAngle.end" = "End";
        /// The Add Constraint form's orientation row.
        orientation: "wideAngle.orientation" = "Orientation";
    }
}
