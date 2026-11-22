use crate::Recording;

/// Decides whether a recording contains any speech at all (energy based),
/// so silence never reaches the speech model.
pub fn contains_speech(recording: &Recording) -> bool {
    todo!()
}
