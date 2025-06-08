use nu_plugin::Plugin;
use nu_plugin::PluginCommand;
use nu_protocol::{
    Category, LabeledError, PipelineData, Record, Signature, Span, SyntaxShape, Type, Value,
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
                    if let ExtraField::ExtendedTimestamp(timestamp_) = field {
                        timestamp = timestamp_.mod_time();
                        break;
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
        debug: bool,
        dir: &Path,
    ) -> Result<PipelineData, LabeledError> {
        for i in 0..archive.len() {
            if let Ok(mut file) = archive.by_index(i) {
                let out_path = match file.enclosed_name() {
                    Some(path) => dir.join(path),
                    None => continue,
                };

                if debug {
                    eprintln!("Extracting {}", out_path.display());
                }

                if out_path.exists() && !force {
                    return Err(LabeledError::new(format!(
                        "File {} already exists",
                        out_path.to_string_lossy()
                    ))
                    .with_label("Use --force/-f to overwrite", span));
                }

                if file.is_dir() {
                    std::fs::create_dir_all(&out_path).map_err(|e| {
                        let out_dir = out_path.to_string_lossy();
                        LabeledError::new(format!("Fail to create {out_dir}"))
                            .with_label(e.to_string(), span)
                    })?;                    
                } else {
                    // are all directories already created ?
                    if let Some(out_dir) = out_path.parent() {
                        std::fs::create_dir_all(out_dir).map_err(|e| {
                            let out_dir = out_dir.to_string_lossy();
                            LabeledError::new(format!("Fail to create {out_dir}"))
                                .with_label(e.to_string(), span)
                        })?;
                    }

                    let mut output_file =
                        std::io::BufWriter::new(std::fs::File::create(&out_path).map_err(|e| {
                            let out_path = out_path.to_string_lossy();
                            LabeledError::new(format!("Fail to create {out_path}"))
                                .with_label(e.to_string(), span)
                        })?);
                    let mut buffer = [0; 1024];
                    loop {
                        let bytes_read = file.read(&mut buffer).map_err(|e| {
                            let file_name = file.name();
                            LabeledError::new(format!("Fail to read {file_name}"))
                                .with_label(e.to_string(), span)
                        })?;
                        if bytes_read == 0 {
                            break;
                        }
                        output_file.write_all(&buffer[0..bytes_read]).map_err(|e| {
                            let out_path = out_path.to_string_lossy();
                            LabeledError::new(format!("Fail to write {out_path}"))
                                .with_label(e.to_string(), span)
                        })?;
                    }
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
            .switch(
                "list",
                "list files in zip file, return table<name, size, modified>",
                Some('l'),
            )
            .switch("force", "force overwrite", Some('f'))
            .switch("debug", "print debug information", None)
            .named(
                "dir",
                SyntaxShape::Directory,
                "the directory to unzip to, default current directory",
                Some('d'),
            )
            .required("file", SyntaxShape::Filepath, "the file to unzip")
            .input_output_types(vec![
                (
                    Type::Nothing,
                    Type::Table(Box::new([
                        ("name".into(), Type::String),
                        ("size".into(), Type::Filesize),
                        ("modified".into(), Type::Date),
                    ])),
                ),
                (Type::Nothing, Type::Nothing),
            ])
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
            let debug = call.has_flag("debug")?;

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
            self.unzip_file(call.head, &mut archive, force, debug, &dir)
        }
    }
}

impl Plugin for UnzipPlugin {
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }

    fn commands(&self) -> Vec<Box<dyn PluginCommand<Plugin = Self>>> {
        vec![Box::new(UnzipCommand)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use chrono::{DateTime, Local};
    use nu_plugin_test_support::PluginTest;
    use nu_protocol::{IntoValue, Record, Value};
    use std::fs;
    use std::fs::File;

    fn make_plugin_with_pwd(pwd: &Path) -> Result<PluginTest> {
        let mut plugin = PluginTest::new("unzip", UnzipPlugin.into())?;

        let pwd = Value::string(pwd.to_string_lossy(), Span::test_data());
        plugin
            .engine_state_mut()
            .add_env_var("PWD".to_string(), pwd);

        Ok(plugin)
    }

    fn make_plugin() -> Result<PluginTest> {
        make_plugin_with_pwd(std::env::temp_dir().as_path())
    }

    // Get the current time
    // convert to zip datetime and back, so that time is truncated as zip datetime
    fn now() -> DateTime<Local> {
        let t = Local::now();
        let zt = zip::DateTime::try_from(t.naive_local()).unwrap();
        let naive_dt: chrono::NaiveDateTime = zt.try_into().unwrap_or_default();
        naive_dt
            .and_local_timezone(Local)
            .single()
            .unwrap_or_default()
    }

    struct TempZipFile {
        _path: PathBuf,
    }

    impl TempZipFile {
        fn new(files: &[(String, Vec<u8>)], modified: DateTime<Local>) -> Result<Self> {
            let path = testfile::generate_name();
            let file = File::create(&path)?;
            let modified = modified.naive_local();

            let mut zip = zip::ZipWriter::new(file);
            for (name, content) in files {
                zip.start_file(
                    name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated)
                        .last_modified_time(modified.try_into()?),
                )?;
                zip.write_all(content)?;
            }
            zip.finish()?;
            Ok(Self { _path: path })
        }

        fn path(&self) -> String {
            self._path.as_path().to_string_lossy().to_string()
        }
    }

    impl Drop for TempZipFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self._path);
        }
    }

    struct TempDir {
        _path: PathBuf,
    }

    impl TempDir {
        fn new() -> Result<Self> {
            let path = testfile::generate_name();
            std::fs::create_dir_all(&path)?;
            Ok(Self { _path: path })
        }

        fn path(&self) -> &Path {
            self._path.as_path()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self._path).unwrap();
        }
    }

    fn make_list_result(files: &[(String, Vec<u8>)], modified: DateTime<Local>) -> Value {
        let items: Vec<_> = files
            .iter()
            .map(|(name, contents)| {
                let item = vec![
                    ("name".to_string(), Value::string(name, Span::test_data())),
                    (
                        "size".to_string(),
                        Value::filesize(contents.len() as i64, Span::test_data()),
                    ),
                    (
                        "modified".to_string(),
                        Value::date(modified.into(), Span::test_data()),
                    ),
                ];
                Record::from_iter(item).into_value(Span::test_data())
            })
            .collect();
        Value::list(items, Span::test_data())
    }

    #[test]
    fn test_not_exists() -> Result<()> {
        let mut plugin = make_plugin()?;

        let not_exists_file = testfile::generate_name();
        let res = plugin.eval(&format!("unzip {}", not_exists_file.to_string_lossy()));

        assert!(res.is_err());
        assert!(res
            .err()
            .unwrap()
            .to_string()
            .contains("Error opening ZIP file"),);

        Ok(())
    }

    #[test]
    fn test_list_empty_zip() -> Result<()> {
        let zip_file = TempZipFile::new(&[], now())?;

        let output = make_plugin()?
            .eval(&format!("unzip -l {}", zip_file.path()))?
            .into_value(Span::test_data())?;

        assert_eq!(output, Value::list(vec![], Span::test_data()));

        Ok(())
    }

    #[test]
    fn test_list_simple_zip() -> Result<()> {
        let files = vec![
            ("file1.txt".to_string(), b"content1".to_vec()),
            ("file2.txt".to_string(), b"hello content2".to_vec()),
        ];
        let modified = now();
        let zip_file = TempZipFile::new(&files, modified)?;

        let output = make_plugin()?
            .eval(&format!("unzip -l {}", zip_file.path()))?
            .into_value(Span::test_data())?;

        assert_eq!(output, make_list_result(&files, modified));

        Ok(())
    }

    #[test]
    fn test_unzip_empty_zip() -> Result<()> {
        let zip_file = TempZipFile::new(&[], now())?;
        let current_dir = TempDir::new()?;

        let output = make_plugin_with_pwd(current_dir.path())?
            .eval(&format!("unzip {}", zip_file.path()))?
            .into_value(Span::test_data())?;

        assert_eq!(output, Value::nothing(Span::test_data()));

        assert!(fs::read_dir(current_dir.path()).unwrap().next().is_none());

        Ok(())
    }

    fn check_extracted_files(files: &[(String, Vec<u8>)], directory: &Path) {
        for (file_name, file_contents) in files {
            let file_path = directory.join(file_name);
            assert!(file_path.exists());
            assert_eq!(
                &fs::read(file_path).unwrap(),
                file_contents,
                "File contents differ for {}",
                file_name
            );
        }
    }

    #[test]
    fn test_unzip_simple_zip() -> Result<()> {
        let files = vec![
            ("file1.txt".to_string(), b"content1".to_vec()),
            ("file2.txt".to_string(), b"hello content2".to_vec()),
        ];
        let modified = now();
        let zip_file = TempZipFile::new(&files, modified)?;
        let current_dir = TempDir::new()?;

        let output = make_plugin_with_pwd(current_dir.path())?
            .eval(&format!("unzip {}", zip_file.path()))?
            .into_value(Span::test_data())?;

        assert_eq!(output, Value::nothing(Span::test_data()));

        check_extracted_files(&files, current_dir.path());

        Ok(())
    }


    #[test]
    fn test_unzip_with_folder() -> Result<()> {
        let files = vec![
            ("file1.txt".to_string(), b"content1".to_vec()),
            ("a_dir/file2.txt".to_string(), b"hello content2".to_vec()),
        ];
        let modified = now();
        let zip_file = TempZipFile::new(&files, modified)?;
        let current_dir = TempDir::new()?;

        let output = make_plugin_with_pwd(current_dir.path())?
            .eval(&format!("unzip {}", zip_file.path()))?
            .into_value(Span::test_data())?;

        assert_eq!(output, Value::nothing(Span::test_data()));

        check_extracted_files(&files, current_dir.path());

        Ok(())
    }
    
    #[test]
    fn test_unzip_force() -> Result<()> {
        let files = vec![
            ("file1.txt".to_string(), b"content1".to_vec()),
            ("file2.txt".to_string(), b"hello content2".to_vec()),
        ];
        let modified = now();
        let zip_file = TempZipFile::new(&files, modified)?;
        let current_dir = TempDir::new()?;

        let mut plugin = make_plugin_with_pwd(current_dir.path())?;

        let cmd = format!("unzip {}", zip_file.path());
        plugin.eval(&cmd)?;

        let res = plugin.eval(&cmd);

        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("already exists"));

        plugin.eval(&(cmd + " -f"))?;

        Ok(())
    }

    #[test]
    fn test_unzip_simple_zip_to_specified_dir() -> Result<()> {
        let files = vec![
            ("file1.txt".to_string(), b"content1".to_vec()),
            ("file2.txt".to_string(), b"hello content2".to_vec()),
        ];
        let modified = now();
        let zip_file = TempZipFile::new(&files, modified)?;
        let current_dir = TempDir::new()?;
        let dest_dir = TempDir::new()?;

        let output = make_plugin_with_pwd(current_dir.path())?
            .eval(&format!(
                "unzip -d {} {}",
                dest_dir.path().to_string_lossy(),
                zip_file.path()
            ))?
            .into_value(Span::test_data())?;

        assert_eq!(output, Value::nothing(Span::test_data()));

        assert!(fs::read_dir(current_dir.path()).unwrap().next().is_none());
        check_extracted_files(&files, dest_dir.path());

        Ok(())
    }
}
