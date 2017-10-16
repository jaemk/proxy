# proxy

[![Build status](https://ci.appveyor.com/api/projects/status/4lkixob628mcx8x2/branch/master?svg=true)](https://ci.appveyor.com/project/jaemk/proxy/branch/master)
[![Build Status](https://travis-ci.org/jaemk/proxy.svg?branch=master)](https://travis-ci.org/jaemk/proxy)
[![crates.io:cli-proxy](https://img.shields.io/crates/v/cli-proxy.svg?label=cli-proxy)](https://crates.io/crates/cli-proxy)

> command-line proxy server

Note, this is intended for development purposes as a quick stand-in for a real proxy server.

## Installation

See [`releases`](https://github.com/jaemk/proxy/releases),

`cargo install cli-proxy`,

Or build from source:
- clone this repo
- `cargo build --release`

Updates:
- Self update functionality (from `github` releases) is available behind `--features update`
- Binary [`releases`](https://github.com/jaemk/proxy/releases) are compiled with the `update` feature
- `proxy self update`

## Usage

```bash
# - proxy requests to `localhost:3002`
# - listen on `localhost:3000`
# - serve requests starting with `/static/` from the relative path `static/`
# - serve requests starting with `/media/`  from the absolute path `/abs/path/to/media
# - serve requests starting with `/assets/` from the relative path `assets`
proxy localhost:3002 --port 3000 --static /static/,static/ --static /media/,/abs/path/to/media --static /assets/,assets
```

