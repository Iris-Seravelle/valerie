//! ANSI styling and color definitions.

pub struct Style;

impl Style {
    pub const RESET: &'static str = "[0m";
    pub const BOLD: &'static str = "[1m";
    pub const DIM: &'static str = "[2m";
    pub const ITALIC: &'static str = "[3m";
    pub const UNDERLINE: &'static str = "[4m";

    // Bold foreground colors (standard Cargo status colors)
    pub const GREEN_BOLD: &'static str = "[1;32m";
    pub const CYAN_BOLD: &'static str = "[1;36m";
    pub const YELLOW_BOLD: &'static str = "[1;33m";
    pub const RED_BOLD: &'static str = "[1;31m";
    pub const BLUE_BOLD: &'static str = "[1;34m";
    pub const MAGENTA_BOLD: &'static str = "[1;35m";
    pub const WHITE_BOLD: &'static str = "[1;37m";

    // Normal foreground
    pub const GREEN: &'static str = "[32m";
    pub const CYAN: &'static str = "[36m";
    pub const YELLOW: &'static str = "[33m";
    pub const RED: &'static str = "[31m";
    pub const BLUE: &'static str = "[34m";
    pub const MAGENTA: &'static str = "[35m";
    pub const WHITE: &'static str = "[37m";
    pub const GRAY: &'static str = "[90m";
}
