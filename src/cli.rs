//! Tools to create command line parsers
//!
//! We can use Schema deinititions to create command line parsers.
//!
//! 

mod environment;
pub use environment::*;

mod format;
pub use format::*;

mod completion;
pub use completion::*;

mod getopts;
pub use getopts::*;

mod command;
pub use command::*;

use std::collections::HashMap;

use proxmox::api::ApiMethod;

pub type CompletionFunction = fn(&str, &HashMap<String, String>) -> Vec<String>;

pub struct CliCommand {
    pub info: &'static ApiMethod,
    pub arg_param: &'static [&'static str],
    pub fixed_param: HashMap<&'static str, String>,
    pub completion_functions: HashMap<String, CompletionFunction>,
}

impl CliCommand {

    pub fn new(info: &'static ApiMethod) -> Self {
        Self {
            info, arg_param: &[],
            fixed_param: HashMap::new(),
            completion_functions: HashMap::new(),
        }
    }

    pub fn arg_param(mut self, names: &'static [&'static str]) -> Self {
        self.arg_param = names;
        self
    }

    pub fn fixed_param(mut self, key: &'static str, value: String) -> Self {
        self.fixed_param.insert(key, value);
        self
    }

    pub fn completion_cb(mut self, param_name: &str, cb:  CompletionFunction) -> Self {
        self.completion_functions.insert(param_name.into(), cb);
        self
    }
}

pub struct CliCommandMap {
    pub commands: HashMap<String, CommandLineInterface>,
}

impl CliCommandMap {

    pub fn new() -> Self {
        Self { commands: HashMap:: new() }
    }

    pub fn insert<S: Into<String>>(mut self, name: S, cli: CommandLineInterface) -> Self {
        self.commands.insert(name.into(), cli);
        self
    }

    fn find_command(&self, name: &str) -> Option<&CommandLineInterface> {

        if let Some(sub_cmd) = self.commands.get(name) {
            return Some(sub_cmd);
        };

        let mut matches: Vec<&str> = vec![];

        for cmd in self.commands.keys() {
            if cmd.starts_with(name) {
                matches.push(cmd); }
        }

        if matches.len() != 1 { return None; }

        if let Some(sub_cmd) = self.commands.get(matches[0]) {
            return Some(sub_cmd);
        };

        None
    }
}

pub enum CommandLineInterface {
    Simple(CliCommand),
    Nested(CliCommandMap),
}

impl From<CliCommand> for CommandLineInterface {
    fn from(cli_cmd: CliCommand) -> Self {
         CommandLineInterface::Simple(cli_cmd)
    }
}

impl From<CliCommandMap> for CommandLineInterface {
    fn from(list: CliCommandMap) -> Self {
        CommandLineInterface::Nested(list)
    }
}
