use super::*;

#[test]
fn text_within_the_limit_is_accepted() {
    let name = VarChar::<5>::new("héllo").expect("five characters fit");

    assert_eq!(name.as_str(), "héllo");
    assert_eq!(&*name, "héllo");
    assert_eq!(name.to_string(), "héllo");
    assert_eq!(name.into_string(), "héllo");
}

#[test]
fn text_over_the_limit_is_rejected_by_characters() {
    assert_eq!(VarChar::<3>::new("héllo"), Err(TooLong { max: 3, len: 5 }));
    assert!(VarChar::<3>::try_from("abc").is_ok());
    assert!(VarChar::<3>::try_from(String::from("abcd")).is_err());
}

#[test]
fn the_column_type_carries_the_length() {
    assert_eq!(
        <VarChar<256> as ScalarColumn>::COLUMN_TYPE,
        ColumnType::VarChar(256)
    );
    assert_eq!(
        <Option<VarChar<64>> as ScalarColumn>::COLUMN_TYPE,
        ColumnType::VarChar(64)
    );
    assert_eq!(<VarChar<8> as ScalarColumn>::SQL_TYPE, SqlType::Text);
}

#[test]
fn values_round_trip_and_oversized_values_fail_to_decode() {
    let name = VarChar::<4>::new("ada").expect("fits");

    assert_eq!(name.to_value(), Value::Text("ada".into()));
    assert_eq!(
        VarChar::<4>::from_value(Value::Text("ada".into())).expect("decodes"),
        name
    );
    assert!(VarChar::<2>::from_value(Value::Text("ada".into())).is_err());
    assert!(VarChar::<4>::from_value(Value::I32(1)).is_err());
}

#[test]
fn serde_writes_a_string_and_checks_the_length_when_reading() {
    let name = VarChar::<4>::new("ada").expect("fits");

    assert_eq!(serde_json::to_string(&name).expect("serializes"), "\"ada\"");
    assert_eq!(
        serde_json::from_str::<VarChar<4>>("\"ada\"").expect("deserializes"),
        name
    );

    let error = serde_json::from_str::<VarChar<2>>("\"ada\"").expect_err("too long");

    assert!(error.to_string().contains("exceeds the limit of 2"));
}
