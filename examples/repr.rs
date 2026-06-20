use matt_db::representations::page::{Leaf};
use matt_db::serialization::{DataType, DataValue, Deserializer, ReadByteStream};

fn main() {
    struct LeafViewer<'a> {
        leaf: &'a Leaf,
        key_types: &'a [DataType],
        value_types: &'a [DataValue]
    }
}