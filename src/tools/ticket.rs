use pbs_api_types::Userid;

pub fn term_aad(userid: &Userid, path: &str, port: u16) -> String {
    format!("{}{}{}", userid, path, port)
}
