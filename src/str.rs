use serde::Serialize;
use sqlx::database::HasValueRef;
use sqlx::error::BoxDynError;
use sqlx::{Database, Decode, Type};
use std::fmt::{Debug, Display, Formatter};

#[derive(Clone, PartialEq, Eq, Hash, Default, Serialize)]
#[repr(transparent)]
pub struct SmolStr(smol_str::SmolStr);

impl Display for SmolStr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl Debug for SmolStr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.0, f)
    }
}

impl SmolStr {
    pub fn new(str: &str) -> Self {
        SmolStr(smol_str::SmolStr::new(str))
    }

    pub const fn new_inline(str: &str) -> Self {
        SmolStr(smol_str::SmolStr::new_inline(str))
    }
}

impl<'a> From<&'a str> for SmolStr {
    fn from(s: &'a str) -> Self {
        SmolStr::new(s)
    }
}

impl<DB: Database> Type<DB> for SmolStr
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

impl<'r, DB> Decode<'r, DB> for SmolStr
where
    DB: Database,
    &'r str: Decode<'r, DB>,
{
    fn decode(value: <DB as HasValueRef<'r>>::ValueRef) -> Result<Self, BoxDynError> {
        let str = <&str as Decode<DB>>::decode(value)?;
        Ok(SmolStr::new(str))
    }
}
