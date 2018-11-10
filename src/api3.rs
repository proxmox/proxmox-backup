use failure::*;
use std::collections::HashMap;


use crate::json_schema::*;
use crate::api_info::*;
use serde_json::{json, Value};

use futures::future::*;
use tokio::prelude::*;
use tokio::fs::File;
use tokio::io;
use tokio_codec;

use hyper::{Method, Body, Request, Response, Server, StatusCode};

fn test_sync_api_handler(param: Value, info: &ApiMethod) -> Result<Value, Error> {
    println!("This is a test {}", param);

   // let force: Option<bool> = Some(false);

    //if let Some(force) = param.force {
    //}

    let _force =  param["force"].as_bool()
        .ok_or_else(|| format_err!("missing parameter 'force'"))?;

    if let Some(_force) = param["force"].as_bool() {
    }


    Ok(json!(null))
}

fn test_async_api_handler(
    param: Value,
    info: &ApiMethod
) -> Box<Future<Item = Response<Body>, Error = Error> + Send> {
    println!("This is a test {}", param);

    let task = lazy(|| {
        println!("A LAZY TASK");


        let dump_file = File::open("/etc/network/interfaces")
            .and_then(|file| {
                let file = std::io::BufReader::new(file);
                let mut linenr = 1;
                tokio::io::lines(file).for_each(move |line| {
                    println!("LINE {}: {}", linenr, line);
                    linenr += 1;
                    ok(())
                })
            }).map_err(|err| ());

        //tokio::spawn(dump_file);

        let dump_file2 = File::open("/etc/network/interfaces")
            .and_then(|file| {
                tokio_codec::FramedRead::new(file, tokio_codec::BytesCodec::new()).for_each(|data| {
                    println!("DATA {:?}", data);
                    ok(())
                })
            }).map_err(|err| ()); // fixme: log error

        tokio::spawn(dump_file2);

        let mut resp = Response::new(Body::from("A LAZY TASKs RESPONSE"));
        *resp.status_mut() = StatusCode::OK;

        ok(resp)
    });

    Box::new(task)
}

pub fn router() -> MethodInfo {

    let route = MethodInfo::new()
        .get(ApiMethod {
            handler: test_sync_api_handler,
            async_handler: test_async_api_handler,
            description: "This is a simple test.",
            parameters: parameter!{
                force => Boolean!{
                    optional => true,
                    description => "Test for boolean options."
                }
            },
            returns: Jss::Null,
        });

    route
}
