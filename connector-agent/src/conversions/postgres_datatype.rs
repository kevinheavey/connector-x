use crate::data_sources::postgres::PostgresDTypes;
use crate::types::DataType;
use crate::typesystem::TypeConversion;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use std::str::from_utf8;

associate_typesystems! {
    (PostgresDTypes, DataType), <'a>,
    ([PostgresDTypes::Float4], [DataType::F64]) => [f32, f64] conversion all,
    ([PostgresDTypes::Float8], [DataType::F64]) => [f64, f64] conversion all,
    ([PostgresDTypes::Int4], [DataType::I64]) => [i32, i64] conversion all,
    ([PostgresDTypes::Int8], [DataType::I64]) => [i64, i64] conversion all,
    ([PostgresDTypes::Bool], [DataType::Bool]) => [bool, bool] conversion all,
    ([PostgresDTypes::Text], [DataType::String]) | ([PostgresDTypes::BpChar], [DataType::String]) | ([PostgresDTypes::VarChar], [DataType::String]) => <'a> [&'a [u8], String] conversion half,
    ([PostgresDTypes::Timestamp], [DataType::DateTime]) => [NaiveDateTime, DateTime<Utc>] conversion half,
    ([PostgresDTypes::TimestampTz], [DataType::DateTime]) => [DateTime<Utc>, DateTime<Utc>] conversion all,
    ([PostgresDTypes::Date], [DataType::DateTime]) => [NaiveDate, DateTime<Utc>] conversion half,
}

impl<'a> TypeConversion<&'a [u8], String> for (PostgresDTypes, DataType) {
    fn convert(val: &'a [u8]) -> String {
        from_utf8(&val[..]).unwrap().to_string()
    }
}

impl TypeConversion<NaiveDateTime, DateTime<Utc>> for (PostgresDTypes, DataType) {
    fn convert(val: NaiveDateTime) -> DateTime<Utc> {
        DateTime::from_utc(val, Utc)
    }
}

impl TypeConversion<NaiveDate, DateTime<Utc>> for (PostgresDTypes, DataType) {
    fn convert(val: NaiveDate) -> DateTime<Utc> {
        DateTime::from_utc(val.and_hms(0, 0, 0), Utc)
    }
}
