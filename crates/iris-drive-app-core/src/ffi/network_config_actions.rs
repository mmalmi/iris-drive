use std::path::Path;

use iris_drive_core::backup_ops::{
    add_blossom_server as core_add_blossom_server,
    remove_blossom_server as core_remove_blossom_server, replace_blossom_servers_in_config,
};
use iris_drive_core::paths::config_path_in;

use super::NativeAppRuntime;

impl NativeAppRuntime {
    pub(super) fn add_blossom_server(&mut self, url: &str) {
        if let Err(error) = core_add_blossom_server(Path::new(&self.data_dir), url) {
            self.state.error = format!("adding Blossom endpoint: {error:#}");
        }
    }

    pub(super) fn remove_blossom_server(&mut self, url: &str) {
        if let Err(error) = core_remove_blossom_server(Path::new(&self.data_dir), url) {
            self.state.error = format!("removing Blossom endpoint: {error:#}");
        }
    }

    pub(super) fn replace_blossom_servers(&mut self, urls: &[String]) {
        let mut config = match self.load_config() {
            Ok(config) => config,
            Err(error) => {
                self.state.error = error;
                return;
            }
        };
        if let Err(error) = replace_blossom_servers_in_config(&mut config, urls) {
            self.state.error = format!("normalizing Blossom endpoints: {error:#}");
            return;
        }
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        }
    }
}
