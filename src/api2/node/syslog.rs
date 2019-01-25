use failure::*;

use crate::tools;
use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};

use std::sync::Arc;
use lazy_static::lazy_static;
use crate::tools::common_regex;

fn get_syslog(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    let result = json!({});

    Ok(result)
}

lazy_static! {
    pub static ref SYSTEMD_DATETIME_FORMAT: Arc<ApiStringFormat> =
        ApiStringFormat::Pattern(&common_regex::SYSTEMD_DATETIME_REGEX).into();
}

pub fn router() -> Router {

    let route = Router::new()
        .get(
            ApiMethod::new(
                get_syslog,
                ObjectSchema::new("Read server time and time zone settings.")
                    .optional(
                        "start",
                        IntegerSchema::new("Start line number.")
                            .minimum(0)
                    )
                    .optional(
                        "limit",
                        IntegerSchema::new("Max. number of lines.")
                            .minimum(0)
                    )
                    .optional(
                        "since",
                        StringSchema::new("Display all log since this date-time string.")
	                    .format(SYSTEMD_DATETIME_FORMAT.clone())
                    )
                    .optional(
                        "until",
                        StringSchema::new("Display all log until this date-time string.")
	                    .format(SYSTEMD_DATETIME_FORMAT.clone())
                    )
                    .optional(
                        "service",
                        StringSchema::new("Service ID.")
                            .max_length(128)
                    )
            ).returns(
                ObjectSchema::new("Returns a list of syslog entries.")
                    .required("n", IntegerSchema::new("Line number."))
                    .required("t", StringSchema::new("Line text."))
            )
        );

    route
}
