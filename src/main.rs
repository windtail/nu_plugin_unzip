use nu_plugin::{serve_plugin, MsgPackSerializer};
use nu_plugin_unzip::UnzipPlugin;

fn main() {
    serve_plugin(&UnzipPlugin {}, MsgPackSerializer {})
}
