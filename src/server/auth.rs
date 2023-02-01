use proxmox_rest_server::AuthError;
use proxmox_router::UserInformation;

use pbs_config::CachedUserInfo;

pub async fn check_pbs_auth(
    headers: &http::HeaderMap,
    method: &hyper::Method,
) -> Result<(String, Box<dyn UserInformation + Sync + Send>), AuthError> {
    let user_info = CachedUserInfo::new()?;
    proxmox_auth_api::api::http_check_auth(headers, method)
        .map(move |name| (name, Box::new(user_info) as _))
}
