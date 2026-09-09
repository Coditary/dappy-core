pub mod debug;
pub mod plugin;
pub mod session;

#[derive(Debug, Clone)]
pub struct GlobalOpts {
    pub json: bool,
    pub control_port: Option<u16>,
    pub scope: Option<String>,
}
