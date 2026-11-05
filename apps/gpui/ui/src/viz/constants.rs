//! Layout metrics and thresholds for soundscape visualization strata.

/// Ridges drawn, bottom of the field upward. Seven is enough to read as layers without turning the
/// window into a bar chart.
pub const STRATA: usize = 7;
/// Points along one ridge. A path is tessellated per frame, so this is a cost as well as a shape.
pub const POINTS: usize = 96;
/// Comets alive at once; a note beyond this replaces the oldest.
pub const COMETS: usize = 10;
/// A ridge is at rest below this, and a level below it means the field is still.
pub const QUIET: f32 = 0.004;
