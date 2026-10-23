//! The heroes' class and colour tables, shared by the simulation and the
//! screens.

/// Class abbreviations by class id (0x801200B0). 16 is Sumner.
pub const CLASSES: [&str; 17] = [
    "WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES", "MIN", "FAL", "JAC", "TIG", "OGR",
    "UNI", "MED", "HYE", "SUM",
];

/// Display names by class id, for the HUD.
pub const HEROES: &[(&str, &str)] = &[
    ("WAR", "WARRIOR"),
    ("VAL", "VALKYRIE"),
    ("WIZ", "WIZARD"),
    ("ARC", "ARCHER"),
    ("DWF", "DWARF"),
    ("KNI", "KNIGHT"),
    ("SOR", "SORCERESS"),
    ("JES", "JESTER"),
    ("MIN", "MINOTAUR"),
    ("FAL", "FALCONESS"),
    ("JAC", "JACKAL"),
    ("TIG", "TIGRESS"),
    ("OGR", "OGRE"),
    ("UNI", "UNICORN"),
    ("MED", "MEDUSA"),
    ("HYE", "HYENA"),
];

/// Costume colours by class_id (0x80120104).
pub const COLOURS: [&str; 4] = ["YEL", "BLU", "RED", "GRE"];
