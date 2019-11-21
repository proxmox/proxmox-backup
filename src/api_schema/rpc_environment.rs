use serde_json::Value;

/// Abstract Interface for API methods to interact with the environment
pub trait RpcEnvironment: std::any::Any + crate::tools::AsAny + Send {

    /// Use this to pass additional result data. It is up to the environment
    /// how the data is used.
    fn set_result_attrib(&mut self, name: &str, value: Value);

    /// Query additional result data.
    fn get_result_attrib(&self, name: &str) -> Option<&Value>;

    /// The environment type
    fn env_type(&self) -> RpcEnvironmentType;

    /// Set user name
    fn set_user(&mut self, user: Option<String>);

    /// Get user name
    fn get_user(&self) -> Option<String>;
}


/// Environment Type
///
/// We use this to enumerate the different environment types. Some methods
/// needs to do different things when started from the command line interface,
/// or when executed from a privileged server running as root.
#[derive(PartialEq, Copy, Clone)]
pub enum RpcEnvironmentType {
    /// Command started from command line
    CLI,
    /// Access from public accessible server
    PUBLIC,
    /// Access from privileged server (run as root)
    PRIVILEGED,
}
