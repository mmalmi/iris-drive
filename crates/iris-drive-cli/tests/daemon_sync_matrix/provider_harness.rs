#[allow(clippy::wildcard_imports)]
use super::*;

impl SyncCluster {
    pub(super) async fn provider_write(&self, client: Client, path: &str, bytes: &[u8]) -> String {
        let previous_root = self.status_current_root(client);
        let log_start = self.daemon_log(client).lines().count();
        let source = self
            .config_path(client)
            .join(format!("provider-source-{}.bin", path_hash_label(path)));
        if let Some(parent) = source.parent() {
            tokio::fs::create_dir_all(parent).await.unwrap();
        }
        tokio::fs::write(&source, bytes).await.unwrap();
        let mut command = idrive(self.config_path(client));
        command.args(["provider", "write", path]).arg(&source);
        let context = format!("provider write {} {path}", client.label());
        self.finish_provider_command(
            client,
            run_checked_json(&mut command, &context),
            previous_root,
            log_start,
            "provider write",
        )
        .await
    }

    pub(super) async fn provider_rename(&self, client: Client, from: &str, to: &str) -> String {
        let previous_root = self.status_current_root(client);
        let log_start = self.daemon_log(client).lines().count();
        let mut command = idrive(self.config_path(client));
        command.args(["provider", "rename", from, to]);
        let context = format!("provider rename {} {from} -> {to}", client.label());
        self.finish_provider_command(
            client,
            run_checked_json(&mut command, &context),
            previous_root,
            log_start,
            "provider rename",
        )
        .await
    }

    pub(super) async fn provider_delete(&self, client: Client, path: &str) -> String {
        let previous_root = self.status_current_root(client);
        let log_start = self.daemon_log(client).lines().count();
        let mut command = idrive(self.config_path(client));
        command.args(["provider", "delete", path]);
        let context = format!("provider delete {} {path}", client.label());
        self.finish_provider_command(
            client,
            run_checked_json(&mut command, &context),
            previous_root,
            log_start,
            "provider delete",
        )
        .await
    }

    pub(super) async fn provider_mkdir(&self, client: Client, path: &str) -> String {
        let previous_root = self.status_current_root(client);
        let log_start = self.daemon_log(client).lines().count();
        let mut command = idrive(self.config_path(client));
        command.args(["provider", "mkdir", path]);
        let context = format!("provider mkdir {} {path}", client.label());
        self.finish_provider_command(
            client,
            run_checked_json(&mut command, &context),
            previous_root,
            log_start,
            "provider mkdir",
        )
        .await
    }

    async fn finish_provider_command(
        &self,
        client: Client,
        value: Value,
        previous_root: Option<String>,
        log_start: usize,
        operation: &str,
    ) -> String {
        let root = self
            .provider_command_published_root(
                client,
                &value,
                previous_root.as_deref(),
                log_start,
                &format!("{} {operation} import", client.label()),
            )
            .await;
        self.refresh_view_until_available(
            client,
            &format!("{} {operation} refresh", client.label()),
        )
        .await;
        root
    }

    async fn provider_command_published_root(
        &self,
        client: Client,
        value: &Value,
        previous_root: Option<&str>,
        log_start: usize,
        label: &str,
    ) -> String {
        let command_root = value["root_cid"].as_str().unwrap().to_string();
        if !value["staged"].as_bool().unwrap_or(false) {
            return command_root;
        }
        self.wait_until(label, || {
            !iris_drive_core::paths::provider_root_staging_path_in(self.config_path(client))
                .exists()
                && self
                    .provider_staged_import_root_after(client, log_start)
                    .as_deref()
                    .is_some_and(|root| Some(root) != previous_root)
        })
        .await;
        self.provider_staged_import_root_after(client, log_start)
            .unwrap_or(command_root)
    }

    fn provider_staged_import_root_after(
        &self,
        client: Client,
        log_start: usize,
    ) -> Option<String> {
        self.daemon_log(client)
            .lines()
            .skip(log_start)
            .find_map(|line| {
                let event = serde_json::from_str::<Value>(line).ok()?;
                (event["event"] == "provider_root_staged_imported")
                    .then(|| event["root_cid"].as_str().map(ToOwned::to_owned))
                    .flatten()
            })
    }

    fn status_current_root(&self, client: Client) -> Option<String> {
        let status = run_json_result(self.config_path(client), &["status"]).ok()?;
        status["hashtree"]["current_root_cid"]
            .as_str()
            .map(ToOwned::to_owned)
    }
}

fn run_checked_json(command: &mut Command, context: &str) -> Value {
    let output = run_command(command, context);
    assert_command_success(&output, context);
    json_output(&output)
}
