//! Manages interactions with Pact plugins
use std::collections::HashMap;
use std::env;
use std::fs;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::Mutex;
use std::thread;

use anyhow::anyhow;
use lazy_static::lazy_static;
use log::{debug, max_level, trace, warn};
use os_info::Type;
use sysinfo::{Pid, ProcessExt, RefreshKind, Signal, System, SystemExt};
use tokio::process::Command;

use crate::catalogue_manager::{register_plugin_entries, remove_plugin_entries};
use crate::child_process::ChildPluginProcess;
use crate::metrics::send_metrics;
use crate::plugin_models::{PactPlugin, PactPluginManifest, PactPluginRpc, PluginDependency};
use crate::proto::InitPluginRequest;

lazy_static! {
  static ref PLUGIN_MANIFEST_REGISTER: Mutex<HashMap<String, PactPluginManifest>> = Mutex::new(HashMap::new());
  static ref PLUGIN_REGISTER: Mutex<HashMap<String, PactPlugin>> = Mutex::new(HashMap::new());
}

/// Load the plugin defined by the dependency information. Will first look in the global
/// plugin registry.
pub async fn load_plugin(plugin: &PluginDependency) -> anyhow::Result<PactPlugin> {
  let thread_id = thread::current().id();
  debug!("Loading plugin {:?}", plugin);
  trace!("Rust plugin driver version {}", option_env!("CARGO_PKG_VERSION").unwrap_or_default());
  trace!("load_plugin {:?}: Waiting on PLUGIN_REGISTER lock", thread_id);
  let mut inner = PLUGIN_REGISTER.lock().unwrap();
  trace!("load_plugin {:?}: Got PLUGIN_REGISTER lock", thread_id);
  let result = match lookup_plugin_inner(plugin, &mut inner) {
    Some(plugin) => {
      debug!("Found running plugin {:?}", plugin);
      plugin.update_access();
      Ok(plugin.clone())
    },
    None => {
      debug!("Did not find plugin, will start it");
      let manifest = load_plugin_manifest(plugin)?;
      send_metrics(&manifest);
      initialise_plugin(&manifest, &mut inner).await
    }
  };
  trace!("load_plugin {:?}: Releasing PLUGIN_REGISTER lock", thread_id);
  result
}

fn lookup_plugin_inner<'a>(
  plugin: &PluginDependency,
  plugin_register: &'a mut HashMap<String, PactPlugin>
) -> Option<&'a mut PactPlugin> {
  if let Some(version) = &plugin.version {
    plugin_register.get_mut(format!("{}/{}", plugin.name, version).as_str())
  } else {
    plugin_register.iter_mut()
      .filter(|(_, value)| value.manifest.name == plugin.name)
      .max_by(|(_, v1), (_, v2)| v1.manifest.version.cmp(&v2.manifest.version))
      .map(|(_, plugin)| plugin)
  }
}

/// Look up the plugin in the global plugin register
pub fn lookup_plugin(plugin: &PluginDependency) -> Option<PactPlugin> {
  let thread_id = thread::current().id();
  trace!("lookup_plugin {:?}: Waiting on PLUGIN_REGISTER lock", thread_id);
  let mut inner = PLUGIN_REGISTER.lock().unwrap();
  trace!("lookup_plugin {:?}: Got PLUGIN_REGISTER lock", thread_id);
  let entry = lookup_plugin_inner(plugin, &mut inner);
  trace!("lookup_plugin {:?}: Releasing PLUGIN_REGISTER lock", thread_id);
  entry.cloned()
}

/// Return the plugin manifest for the given plugin. Will first look in the global plugin manifest
/// registry.
pub fn load_plugin_manifest(plugin_dep: &PluginDependency) -> anyhow::Result<PactPluginManifest> {
  debug!("Loading plugin manifest for plugin {:?}", plugin_dep);
  match lookup_plugin_manifest(plugin_dep) {
    Some(manifest) => Ok(manifest),
    None => load_manifest_from_disk(plugin_dep)
  }
}

fn load_manifest_from_disk(plugin_dep: &PluginDependency) -> anyhow::Result<PactPluginManifest> {
  let plugin_dir = pact_plugin_dir()?;
  debug!("Looking for plugin in {:?}", plugin_dir);

  if plugin_dir.exists() {
    for entry in fs::read_dir(plugin_dir)? {
      let path = entry?.path();
      trace!("Found: {:?}", path);

      if path.is_dir() {
        let manifest_file = path.join("pact-plugin.json");
        if manifest_file.exists() && manifest_file.is_file() {
          debug!("Found plugin manifest: {:?}", manifest_file);
          let file = File::open(manifest_file)?;
          let reader = BufReader::new(file);
          let manifest: PactPluginManifest = serde_json::from_reader(reader)?;
          trace!("Parsed plugin manifest: {:?}", manifest);
          let version = manifest.version.clone();
          if manifest.name == plugin_dep.name && (plugin_dep.version.is_none() ||
            plugin_dep.version.as_ref().unwrap() == &version) {
            let manifest = PactPluginManifest {
              plugin_dir: path.to_string_lossy().to_string(),
              ..manifest
            };
            let key = format!("{}/{}", manifest.name, version);
            {
              let manifest = manifest.clone();
              let mut guard = PLUGIN_MANIFEST_REGISTER.lock().unwrap();
              guard.insert(key.clone(), manifest.clone());
            }
            return Ok(manifest);
          }
        }
      }
    }
    Err(anyhow!("Plugin {:?} was not found (in $HOME/.pact/plugins or $PACT_PLUGIN_DIR)", plugin_dep))
  } else {
    Err(anyhow!("Plugin directory {:?} does not exist", plugin_dir))
  }
}

fn pact_plugin_dir() -> anyhow::Result<PathBuf> {
  let env_var = env::var_os("PACT_PLUGIN_DIR");
  let plugin_dir = env_var.unwrap_or_default();
  let plugin_dir = plugin_dir.to_string_lossy();
  if plugin_dir.is_empty() {
    home::home_dir().map(|dir| dir.join(".pact/plugins"))
  } else {
    PathBuf::from_str(plugin_dir.as_ref()).ok()
  }.ok_or_else(|| anyhow!("No Pact plugin directory was found (in $HOME/.pact/plugins or $PACT_PLUGIN_DIR)"))
}

/// Lookup the plugin manifest in the global plugin manifest registry.
pub fn lookup_plugin_manifest(plugin: &PluginDependency) -> Option<PactPluginManifest> {
  let guard = PLUGIN_MANIFEST_REGISTER.lock().unwrap();
  if let Some(version) = &plugin.version {
    let key = format!("{}/{}", plugin.name, version);
    guard.get(&key).cloned()
  } else {
    guard.iter()
      .filter(|(_, value)| value.name == plugin.name)
      .max_by(|(_, v1), (_, v2)| v1.version.cmp(&v2.version))
      .map(|(_, p)| p.clone())
  }
}

async fn initialise_plugin(
  manifest: &PactPluginManifest,
  plugin_register: &mut HashMap<String, PactPlugin>
) -> anyhow::Result<PactPlugin> {
  match manifest.executable_type.as_str() {
    "exec" => {
      let plugin = start_plugin_process(manifest).await?;
      debug!("Plugin process started OK (port = {}), sending init message", plugin.port());

      init_handshake(manifest, &plugin).await.map_err(|err| {
        plugin.kill();
        anyhow!("Failed to send init request to the plugin - {}", err)
      })?;

      let key = format!("{}/{}", manifest.name, manifest.version);
      plugin_register.insert(key, plugin.clone());

      Ok(plugin)
    }
    _ => Err(anyhow!("Plugin executable type of {} is not supported", manifest.executable_type))
  }
}

/// Internal function: public for testing
pub async fn init_handshake(manifest: &PactPluginManifest, plugin: &dyn PactPluginRpc) -> anyhow::Result<()> {
  let request = InitPluginRequest {
    implementation: "plugin-driver-rust".to_string(),
    version: option_env!("CARGO_PKG_VERSION").unwrap_or("0").to_string()
  };
  let response = plugin.init_plugin(request).await?;
  debug!("Got init response {:?} from plugin {}", response, manifest.name);
  register_plugin_entries(manifest, &response.catalogue);
  tokio::task::spawn(async { publish_updated_catalogue() });
  Ok(())
}

async fn start_plugin_process(manifest: &PactPluginManifest) -> anyhow::Result<PactPlugin> {
  debug!("Starting plugin with manifest {:?}", manifest);

  let os_info = os_info::get();
  debug!("Detected OS: {}", os_info);
  let mut path = if let Some(entry_point) = manifest.entry_points.get(&os_info.to_string()) {
    PathBuf::from(entry_point)
  } else if os_info.os_type() == Type::Windows && manifest.entry_points.contains_key("windows") {
    PathBuf::from(manifest.entry_points.get("windows").unwrap())
  } else {
    PathBuf::from(&manifest.entry_point)
  };

  if !path.is_absolute() || !path.exists() {
    path = PathBuf::from(manifest.plugin_dir.clone()).join(path);
  }
  debug!("Starting plugin using {:?}", path);

  let log_level = max_level();
  let child = Command::new(path)
    .env("LOG_LEVEL", log_level.as_str())
    .env("RUST_LOG", log_level.as_str())
    .current_dir(manifest.plugin_dir.clone())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;
  let child_pid = child.id().unwrap_or_default();
  debug!("Plugin {} started with PID {}", manifest.name, child_pid);

  match ChildPluginProcess::new(child, manifest).await {
    Ok(child) => Ok(PactPlugin::new(manifest, child)),
    Err(err) => {
      let s = System::new_with_specifics(RefreshKind::new().with_processes());
      if let Some(process) = s.process(child_pid as Pid) {
        process.kill(Signal::Term);
      } else {
        warn!("Child process with PID {} was not found", child_pid);
      }
      Err(err)
    }
  }
}

/// Shut down all plugin processes
pub fn shutdown_plugins() {
  let thread_id = thread::current().id();
  debug!("Shutting down all plugins");
  trace!("shutdown_plugins {:?}: Waiting on PLUGIN_REGISTER lock", thread_id);
  let mut guard = PLUGIN_REGISTER.lock().unwrap();
  trace!("shutdown_plugins {:?}: Got PLUGIN_REGISTER lock", thread_id);
  for plugin in guard.values() {
    debug!("Shutting down plugin {:?}", plugin);
    plugin.kill();
    remove_plugin_entries(&plugin.manifest.name);
  }
  guard.clear();
  trace!("shutdown_plugins {:?}: Releasing PLUGIN_REGISTER lock", thread_id);
}

/// Shutdown the given plugin
pub fn shutdown_plugin(plugin: &mut PactPlugin) {
  debug!("Shutting down plugin {}:{}", plugin.manifest.name, plugin.manifest.version);
  plugin.kill();
  remove_plugin_entries(&plugin.manifest.name);
}

// TODO
fn publish_updated_catalogue() {
  // val requestBuilder = Plugin.Catalogue.newBuilder()
  // CatalogueManager.entries().forEach { (_, entry) ->
  //   requestBuilder.addCatalogue(Plugin.CatalogueEntry.newBuilder()
  //     .setKey(entry.key)
  //     .setType(entry.type.name)
  //     .putAllValues(entry.values)
  //     .build())
  // }
  // val request = requestBuilder.build()
  //
  // PLUGIN_REGISTER.forEach { (_, plugin) ->
  //   plugin.stub?.updateCatalogue(request)
  // }
}

/// Decrement access to the plugin. If the current access could is zero, shut down the plugin
pub fn drop_plugin_access(plugin: &PluginDependency) {
  let thread_id = thread::current().id();

  trace!("drop_plugin_access {:?}: Waiting on PLUGIN_REGISTER lock", thread_id);
  let mut inner = PLUGIN_REGISTER.lock().unwrap();
  trace!("drop_plugin_access {:?}: Got PLUGIN_REGISTER lock", thread_id);

  if let Some(plugin) = lookup_plugin_inner(plugin, &mut inner) {
    let key = format!("{}/{}", plugin.manifest.name, plugin.manifest.version);
    if plugin.drop_access() == 0 {
      shutdown_plugin(plugin);
      inner.remove(key.as_str());
    }
  }

  trace!("drop_plugin_access {:?}: Releasing PLUGIN_REGISTER lock", thread_id);
}
