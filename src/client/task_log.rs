use failure::*;
use serde_json::json;

use super::HttpClient;

pub async fn display_task_log(
    client: HttpClient,
    upid_str: &str,
    strip_date: bool,
) -> Result<(), Error> {

    let path = format!("api2/json/nodes/localhost/tasks/{}/log", upid_str);

    let mut start = 1;
    let limit = 500;

    loop {
        let param = json!({ "start": start, "limit": limit, "test-status": true });
        let result = client.get(&path, Some(param)).await?;

        let active = result["active"].as_bool().unwrap();
        let total = result["total"].as_u64().unwrap();
        let data = result["data"].as_array().unwrap();

        let lines = data.len();

        for item in data {
            let n = item["n"].as_u64().unwrap();
            let t = item["t"].as_str().unwrap();
            if n != start { bail!("got wrong line number in response data ({} != {}", n, start); }
             if strip_date && t.len() > 27 && &t[25..27] == ": " {
                let line = &t[27..];
                println!("{}", line);
            } else {
                println!("{}", t);
            }
            start += 1;
       }

        if start > total {
            if active {
                std::thread::sleep(std::time::Duration::from_millis(1000));
            } else {
                break;
            }
        } else {
            if lines != limit { bail!("got wrong number of lines from server ({} != {})", lines, limit); }
        }
    }

    Ok(())
}
