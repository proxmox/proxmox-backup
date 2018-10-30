#![feature(plugin)]
#![plugin(phf_macros)]
extern crate phf;

extern crate failure;

extern crate json_schema;
use json_schema::*;


static PARAMETERS1: StaticPropertyMap = phf_map! {
    "force" => Boolean!{
        description => "Test for boolean options."
    },
    "text1" => ApiString!{
        description => "A simple text string.",
        min_length => Some(10),
        max_length => Some(30)
    },
    "count" => Integer!{
        description => "A counter for everything.",
        minimum => Some(0),
        maximum => Some(10)
    },
    "myarray1" => Array!{
        description => "Test Array of simple integers.",
        items => &PVE_VMID
    },
    "myarray2" => Jss::Array(JssArray {
        description: "Test Array of simple integers.",
        optional: Some(false),
        items: &Object!{description => "Empty Object."},
    }),
    "myobject" => Object!{
        description => "TEST Object.",
        properties => &phf_map!{
            "vmid" => Jss::Reference { reference: &PVE_VMID},
            "loop" => Integer!{
                description => "Totally useless thing.",
                optional => Some(false)
            }
        }
    },
    "emptyobject" => Object!{description => "Empty Object."},
};


fn main() {
    println!("Fast Static Type Definitions 1");

    for (k, v) in PARAMETERS1.entries() {
        println!("Parameter: {} Value: {:?}", k, v);
    }
    
}
