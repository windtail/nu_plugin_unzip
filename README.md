## nu_plugin_unzip

A [nushell](https://www.nushell.sh/) plugin for unzipping files.

### Installation

```shell
cargo install nu_plugin_unzip
plugin add ~/.cargo/bin/nu_plugin_unzip
plugin use unzip
```

### Usage

```shell
unzip -l a.zip  # list contents of zip file
```

```shell
unzip a.zip # unzip file to current directory
unzip -f a.zip # unzip file to current directory, overwriting existing files
unzip -d /tmp a.zip # unzip file to /tmp
```
