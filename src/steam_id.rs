use serde::{Serialize, Serializer};
use sqlx::error::BoxDynError;
use sqlx::{Database, Decode, Type};
use std::convert::TryFrom;
use std::fmt::{Debug, Formatter};
use std::str::FromStr;
use steamid_ng::SteamID;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct SteamId(u64);

impl SteamId {
    pub const fn new(id: u64) -> SteamId {
        SteamId(id)
    }

    pub fn steam3(&self) -> String {
        SteamID::from(self.0).steam3()
    }

    pub fn steam2(&self) -> String {
        SteamID::from(self.0).steam2()
    }

    pub fn from_steam3(s: &str) -> Result<Self, steamid_ng::SteamIDError> {
        let id = SteamID::from_steam3(s)?;
        Ok(SteamId(id.into()))
    }
}

impl Debug for SteamId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        SteamID::from(self.0).fmt(f)
    }
}

impl Serialize for SteamId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&self.0)
    }
}

impl From<SteamID> for SteamId {
    fn from(id: SteamID) -> Self {
        SteamId(id.into())
    }
}

impl From<u64> for SteamId {
    fn from(id: u64) -> Self {
        SteamId(id)
    }
}

impl From<SteamId> for u64 {
    fn from(id: SteamId) -> Self {
        id.0
    }
}

impl FromStr for SteamId {
    type Err = steamid_ng::SteamIDError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let id = SteamID::try_from(s)?;
        Ok(SteamId(id.into()))
    }
}

impl<DB: Database> Type<DB> for SteamId
where
    i64: Type<DB>,
    str: Type<DB>,
{
    fn type_info() -> DB::TypeInfo {
        <str as Type<DB>>::type_info()
    }

    fn compatible(ty: &DB::TypeInfo) -> bool {
        <str as Type<DB>>::compatible(ty)
    }
}

impl<'r, DB> Decode<'r, DB> for SteamId
where
    DB: Database,
    &'r str: Decode<'r, DB>,
{
    fn decode(value: DB::ValueRef<'r>) -> Result<Self, BoxDynError> {
        let str = <&str as Decode<DB>>::decode(value)?;
        Ok(Self::from_steam3(str)?)
    }
}
