#![feature(plugin)]
#![plugin(phf_macros)]

extern crate failure;

extern crate phf;

extern crate serde_json;

// Jss => JavaScript Schema

//use failure::Error;


pub mod json_schema;
pub mod api_info;
