//! Small value types shared across the engine.

use serde::{Deserialize, Serialize};
use std::fmt;

/// An asset token. The first twelve are standard event assets; the last three
/// are special tokens drawn from the bag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Asset {
    Gladiator,
    Comedian,
    Musician,
    Horse,
    Torch,
    Priest,
    Ship,
    Lion,
    Scenery,
    Decoration,
    Chariot,
    Cage,
    Joker,
    Emperor,
    SpecialEvent,
}

impl Asset {
    pub const COUNT: usize = 15;
    pub const ALL: [Asset; Asset::COUNT] = [
        Asset::Gladiator,
        Asset::Comedian,
        Asset::Musician,
        Asset::Horse,
        Asset::Torch,
        Asset::Priest,
        Asset::Ship,
        Asset::Lion,
        Asset::Scenery,
        Asset::Decoration,
        Asset::Chariot,
        Asset::Cage,
        Asset::Joker,
        Asset::Emperor,
        Asset::SpecialEvent,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// Standard event assets: the ones events require and players may request
    /// as a donation.
    pub fn is_standard(self) -> bool {
        !matches!(self, Asset::Joker | Asset::Emperor | Asset::SpecialEvent)
    }

    /// Assets that may change hands in the trading phase.
    pub fn is_tradeable(self) -> bool {
        self.is_standard() || self == Asset::Joker
    }

    pub fn name(self) -> &'static str {
        match self {
            Asset::Gladiator => "Gladiator",
            Asset::Comedian => "Comedian",
            Asset::Musician => "Musician",
            Asset::Horse => "Horse",
            Asset::Torch => "Torch",
            Asset::Priest => "Priest",
            Asset::Ship => "Ship",
            Asset::Lion => "Lion",
            Asset::Scenery => "Scenery",
            Asset::Decoration => "Decoration",
            Asset::Chariot => "Chariot",
            Asset::Cage => "Cage",
            Asset::Joker => "Joker",
            Asset::Emperor => "Emperor",
            Asset::SpecialEvent => "SpecialEvent",
        }
    }

    /// Case-insensitive parse that also accepts a trailing plural "s".
    pub fn parse(s: &str) -> Option<Asset> {
        let s = s.trim().to_ascii_lowercase();
        Asset::ALL.into_iter().find(|a| {
            let n = a.name().to_ascii_lowercase();
            s == n || s == format!("{n}s")
        })
    }
}

impl fmt::Display for Asset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A multiset of assets, stored as a count per asset type. It serializes as
/// a map of the non-zero counts, like `{"Gladiator": 2, "Horse": 1}`.
#[derive(Clone, Default, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "std::collections::BTreeMap<Asset, u32>", into = "std::collections::BTreeMap<Asset, u32>")]
pub struct Assets {
    counts: [u32; Asset::COUNT],
}

impl Assets {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(&self, asset: Asset) -> u32 {
        self.counts[asset.index()]
    }

    pub fn add(&mut self, asset: Asset, n: u32) {
        self.counts[asset.index()] += n;
    }

    /// Removes `n` of `asset`. Returns false (and changes nothing) if there
    /// are not enough.
    pub fn remove(&mut self, asset: Asset, n: u32) -> bool {
        let c = &mut self.counts[asset.index()];
        if *c < n {
            return false;
        }
        *c -= n;
        true
    }

    pub fn total(&self) -> u32 {
        self.counts.iter().sum()
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }

    pub fn contains_all(&self, other: &Assets) -> bool {
        self.counts.iter().zip(other.counts.iter()).all(|(a, b)| a >= b)
    }

    pub fn add_all(&mut self, other: &Assets) {
        for (a, b) in self.counts.iter_mut().zip(other.counts.iter()) {
            *a += b;
        }
    }

    /// Removes every asset in `other`. Returns false (and changes nothing) if
    /// any are missing.
    pub fn remove_all(&mut self, other: &Assets) -> bool {
        if !self.contains_all(other) {
            return false;
        }
        for (a, b) in self.counts.iter_mut().zip(other.counts.iter()) {
            *a -= b;
        }
        true
    }

    /// Non-zero `(asset, count)` pairs in asset order.
    pub fn iter(&self) -> impl Iterator<Item = (Asset, u32)> + '_ {
        Asset::ALL
            .into_iter()
            .map(|a| (a, self.count(a)))
            .filter(|(_, n)| *n > 0)
    }

    /// One entry per token, in asset order.
    pub fn to_vec(&self) -> Vec<Asset> {
        self.iter()
            .flat_map(|(a, n)| std::iter::repeat_n(a, n as usize))
            .collect()
    }
}

impl From<std::collections::BTreeMap<Asset, u32>> for Assets {
    fn from(map: std::collections::BTreeMap<Asset, u32>) -> Self {
        let mut out = Assets::new();
        for (a, n) in map {
            out.add(a, n);
        }
        out
    }
}

impl From<Assets> for std::collections::BTreeMap<Asset, u32> {
    fn from(assets: Assets) -> Self {
        assets.iter().collect()
    }
}

impl FromIterator<Asset> for Assets {
    fn from_iter<I: IntoIterator<Item = Asset>>(iter: I) -> Self {
        let mut out = Assets::new();
        for a in iter {
            out.add(a, 1);
        }
        out
    }
}

impl fmt::Display for Assets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("nothing");
        }
        let parts: Vec<String> = self.iter().map(|(a, n)| format!("{n} {a}")).collect();
        f.write_str(&parts.join(", "))
    }
}

/// Seat index of a player, `0..num_players`, in clockwise order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PlayerId(pub usize);

impl fmt::Display for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "P{}", self.0 + 1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NobleKind {
    Emperor,
    Consul,
    Senator,
}

impl fmt::Display for NobleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NobleKind::Emperor => "Emperor",
            NobleKind::Consul => "Consul",
            NobleKind::Senator => "Senator",
        })
    }
}

/// Direction to grow an arena along the clockwise track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    /// Counter-clockwise end.
    Left,
    /// Clockwise end.
    Right,
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Side::Left => "left",
            Side::Right => "right",
        })
    }
}
