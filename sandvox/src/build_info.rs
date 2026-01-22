#[derive(Clone, Debug)]
pub struct BuildInfo {
    pub target: &'static str,
    pub opt_level: &'static str,
    pub debug: &'static str,
    pub profile: &'static str,
    pub git_commit: Option<&'static str>,
    pub git_branch: Option<&'static str>,
    pub version: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/build_info.rs"));
