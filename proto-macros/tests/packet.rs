extern crate self as coeus;

mod common {
    pub mod proto {
        pub use coeus_proto_macros::Packet;

        pub enum Packet {
            MyPacket(Box<crate::MyPacket>),
            Message(Box<crate::Message>),
            Tuple(Box<crate::Tuple>),
            Unit(Box<crate::Unit>),
        }
    }
}

use common::proto::Packet;

#[derive(Packet)]
pub struct MyPacket {}

#[derive(Packet)]
pub struct Message {
    text: String,
}

#[derive(Packet)]
pub struct Tuple(u16);

#[derive(Packet)]
pub struct Unit;

#[test]
fn converts_empty_struct() {
    let packet: Packet = MyPacket {}.into();
    let Packet::MyPacket(value) = packet else {
        panic!("converted to the wrong packet variant");
    };
    let MyPacket {} = *value;
}

#[test]
fn preserves_named_fields() {
    let packet = Packet::from(Message {
        text: "hello".into(),
    });
    let Packet::Message(value) = packet else {
        panic!("converted to the wrong packet variant");
    };
    assert_eq!(value.text, "hello");
}

#[test]
fn preserves_tuple_fields() {
    let packet: Packet = Tuple(42).into();
    let Packet::Tuple(value) = packet else {
        panic!("converted to the wrong packet variant");
    };
    assert_eq!(value.0, 42);
}

#[test]
fn converts_unit_struct() {
    let packet = Packet::from(Unit);
    let Packet::Unit(value) = packet else {
        panic!("converted to the wrong packet variant");
    };
    let Unit = *value;
}
