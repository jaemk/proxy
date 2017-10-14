# proxy [![Build Status](https://travis-ci.org/jaemk/proxy.svg?branch=master)](https://travis-ci.org/jaemk/proxy)

> command line proxy server

## Installation

See [`releases`](https://github.com/jaemk/proxy/releases)

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
# - serve requests starting with `/media/` from the absolute path `/abs/path/to/media
# - serve requests starting with `/assets/` from the relative path `assets`
proxy localhost:3002 --port 3000 --static /static/,static/ --static /media/,/abs/path/to/media --static /assets/,assets
```

