//! Built-in words of the form dialogs family.

craft_ui::words! {
    /// The words of the parameter rows: the buttons and names a row adds beside its control. Each id is prefixed by
    /// its module. `{name}` stands for the row's label, `{row}` and `{column}` for a grid cell's place.
    pub struct ParamWords {
        browse: "param.browse" = "Browse…";
        randomize: "param.randomize" = "Random";
        eyedropper: "param.eyedropper" = "Eyedropper";
        custom: "param.custom" = "Custom";
        unit: "param.unit" = "Unit";
        angle_field: template "param.angleField" = "{name} in degrees";
        point_x: template "param.pointX" = "{name} X";
        point_y: template "param.pointY" = "{name} Y";
        grid_cell: template "param.gridCell" = "{name} {row}, {column}";
        choose_color: "param.chooseColor" = "Choose…";
        color_unset: "param.colorUnset" = "Not set";
    }
}
