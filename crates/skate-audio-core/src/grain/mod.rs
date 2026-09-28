//! Retail board rolling: the granular player over `data\audio\grains.big`.
//!
//! Retail's continuous rolling sound is not the `Class_rolling` patch but two `GrainPlayer`s per
//! truck owned by `SFXObj_SkateBoard` (`docs/player-audio-retail-drivers.md` §3). Each plays
//! short windows of one 12–22 s `.grain` recording through two voice graphs of its own,
//! crossfading attack/sustain/release from a scheduler plug-in that runs every audio block, and
//! sends them into its own bus chain.
//!
//! | module | retail |
//! |---|---|
//! | [`player`] | the 372-byte `GrainPlayer` (`sub_828EBD88` … `sub_828EC6F0`), on guest memory |
//! | [`rng`] | the title's shared generator `sub_82A8AF10`, which draws each grain |
//! | [`seek`] | the SndPlayer1 seek-table reader `sub_82B470D0` and the start-frame arithmetic |
//! | [`chain`] | the per-player bus chain `sub_824C8878` builds and `sub_824C9058` drives |
//! | [`fss`] | the chain's `FrequencyShiftSsb` and `HighShelfIir2` module classes |
//! | [`board`] | the board owner's surface → grain choice, per-frame record, slew, chain values, latches |
//! | [`envelope`] | the owner's push envelopes (`owner+912`/`+1036`: the speed scale and shift offset) |
//! | [`host`] | what a runtime owner adds to host players: phase-1 tick, resident seek, the game API |

pub mod board;
pub mod chain;
pub mod envelope;
pub mod fss;
pub mod host;
pub mod player;
pub mod rng;
pub mod seek;
pub mod stream;
