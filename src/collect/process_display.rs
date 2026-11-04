// Modified for rztop on 2026-11-04: kept only ProcSort; the process tree and filter went with the terminal frontend.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("invalid ProcSort value '{0}'")]
pub struct ParseProcSortError(pub String);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProcSort {
    Pid,
    Name,
    Command,
    Threads,
    User,
    Memory,
    #[default]
    Cpu,
}

impl ProcSort {
    pub const ALL: &'static [ProcSort] = &[
        ProcSort::Pid,
        ProcSort::Name,
        ProcSort::Command,
        ProcSort::Threads,
        ProcSort::User,
        ProcSort::Memory,
        ProcSort::Cpu,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ProcSort::Pid => "pid",
            ProcSort::Name => "name",
            ProcSort::Command => "command",
            ProcSort::Threads => "threads",
            ProcSort::User => "user",
            ProcSort::Memory => "memory",
            ProcSort::Cpu => "cpu",
        }
    }
}

impl fmt::Display for ProcSort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProcSort {
    type Err = ParseProcSortError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ProcSort::ALL
            .iter()
            .copied()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| ParseProcSortError(s.to_string()))
    }
}

impl Serialize for ProcSort {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ProcSort {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = <String>::deserialize(d)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}
