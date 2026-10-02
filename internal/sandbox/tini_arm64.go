package sandbox

import _ "embed"

// tini is the static tini binary that runs as the init process of a sandbox.
//
//go:embed tini/tini-static-arm64
var tini []byte
