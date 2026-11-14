//! Mac-formatted disks (HFS+, APFS), read-only and in-process. On Windows they have no
//! drive letter, but an unelevated process may read their partition device; see the
//! architecture spec §8.4.

pub mod aligned;
pub mod fs;
pub mod probe;
pub mod source;
