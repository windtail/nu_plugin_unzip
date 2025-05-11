use nu_plugin::Plugin;
use nu_plugin::PluginCommand;
use nu_protocol::{
    Category, LabeledError, PipelineData, Record, Signature, Span, SyntaxShape, Type,
    Value,
};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::{ExtraField, ZipArchive};

pub struct UnzipPlugin;

pub struct UnzipCommand;

impl UnzipCommand {
    fn list_files(
        &self,
        span: Span,
        archive: &mut ZipArchive<std::fs::File>,
    ) -> Result<PipelineData, LabeledError> {
        let mut rows = Vec::new();
        for i in 0..archive.len() {
            if let Ok(file) = archive.by_index(i) {
                let file_name = file.name();
                let uncompressed_size = file.size();

                let mut timestamp = None;
                for field in file.extra_data_fields() {
                    match field {
                        ExtraField::ExtendedTimestamp(timestamp_) => {
                            timestamp = timestamp_.mod_time();
                            break
                        }
                        _ => {}
                    }
                }
                let last_modified: chrono::DateTime<chrono::Local> = match timestamp {
                    Some(timestamp) => chrono::DateTime::from_timestamp(timestamp as i64, 0)
                        .unwrap_or_default()
                        .into(),
                    None => {
                        let zip_dt = file.last_modified().unwrap_or_default();
                        let naive_dt: chrono::NaiveDateTime = zip_dt.try_into().unwrap_or_default();
                        naive_dt
                            .and_local_timezone(chrono::Local)
                            .single()
                            .unwrap_or_default()
                    }
                };

                let mut row = Record::default();
                row.push("name", Value::string(file_name, span));
                row.push("size", Value::filesize(uncompressed_size as i64, span));
                row.push("modified", Value::date(last_modified.into(), span));

                rows.push(Value::record(row, span));
            }
        }

        Ok(PipelineData::Value(Value::list(rows, span), None))
    }

    fn unzip_file(
        &self,
        span: Span,
        archive: &mut ZipArchive<std::fs::File>,
        force: bool,
        dir: &Path,
    ) -> Result<PipelineData, LabeledError> {
        for i in 0..archive.len() {
            if let Ok(mut file) = archive.by_index(i) {
                let out_path = match file.enclosed_name() {
                    Some(path) => dir.join(path),
                    None => continue,
                };

                if out_path.exists() && !force {
                    return Err(LabeledError::new(format!(
                        "File {} already exists",
                        out_path.to_string_lossy()
                    ))
                    .with_label("Use --force/-f to overwrite", span));
                }

                if let Some(out_dir) = out_path.parent() {
                    std::fs::create_dir_all(out_dir).map_err(|e| {
                        let out_dir = out_dir.to_string_lossy();
                        LabeledError::new(format!("Fail to create {out_dir}")).with_label(e.to_string(), span)
                    })?;
                }

                let mut output_file = std::io::BufWriter::new(
                    std::fs::File::create(&out_path).map_err(|e| {
                        let out_path = out_path.to_string_lossy();
                        LabeledError::new(format!("Fail to create {out_path}")).with_label(e.to_string(), span)
                    })?,
                );
                let mut buffer = [0; 1024];
                loop {
                    let bytes_read = file.read(&mut buffer).map_err(|e| {
                        let file_name = file.name();
                        LabeledError::new(format!("Fail to read {file_name}")).with_label(e.to_string(), span)
                    })?;
                    if bytes_read == 0 {
                        break;
                    }
                    output_file
                        .write_all(&buffer[0..bytes_read])
                        .map_err(|e| {
                            let out_path = out_path.to_string_lossy();
                            LabeledError::new(format!("Fail to write {out_path}")).with_label(e.to_string(), span)
                        })?;
                }
            }
        }

        Ok(PipelineData::Value(Value::nothing(span), None))
    }
}

impl PluginCommand for UnzipCommand {
    type Plugin = UnzipPlugin;

    fn name(&self) -> &str {
        "unzip"
    }

    fn signature(&self) -> Signature {
        Signature::build("unzip")
            .switch("list", "list files in zip file", Some('l'))
            .switch("force", "force overwrite", Some('f'))
            .named(
                "dir",
                SyntaxShape::Filepath,
                "the directory to unzip to, default current directory",
                Some('d'),
            )
            .required("file", SyntaxShape::Filepath, "the file to unzip")
            .input_output_types(vec![(
                Type::Nothing,
                Type::Table(Box::new([
                    ("name".into(), Type::String),
                    ("size".into(), Type::Filesize),
                    ("modified".into(), Type::Date),
                ])),
            )])
            .allow_variants_without_examples(true)
            .category(Category::FileSystem)
            .filter()
    }

    fn description(&self) -> &str {
        "unzip file"
    }

    fn run(
        &self,
        _plugin: &Self::Plugin,
        engine: &nu_plugin::EngineInterface,
        call: &nu_plugin::EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        let zip_file_path = call.req::<PathBuf>(0)?;
        let zip_file_path = if zip_file_path.is_relative() {
            let current_dir = std::path::PathBuf::from(engine.get_current_dir()?);
            current_dir.join(zip_file_path)
        } else {
            zip_file_path
        };

        let zip_file = std::fs::File::open(zip_file_path).map_err(|e| {
            LabeledError::new("Error opening ZIP file").with_label(e.to_string(), call.head)
        })?;

        let mut archive = ZipArchive::new(zip_file).map_err(|e| {
            LabeledError::new("Error reading ZIP file").with_label(e.to_string(), call.head)
        })?;

        let list_only = call.has_flag("list")?;
        if list_only {
            self.list_files(call.head, &mut archive)
        } else {
            let force = call.has_flag("force")?;

            let current_dir: PathBuf = engine.get_current_dir()?.into();
            let dir = call
                .get_flag::<PathBuf>("dir")?
                .map(|p| {
                    if p.is_relative() {
                        current_dir.join(p)
                    } else {
                        p
                    }
                })
                .unwrap_or_else(|| current_dir);
            self.unzip_file(call.head, &mut archive, force, &dir)
        }
    }
}

impl Plugin for UnzipPlugin {
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }

    fn commands(&self) -> Vec<Box<dyn nu_plugin::PluginCommand<Plugin = Self>>> {
        vec![Box::new(UnzipCommand)]
    }
}
