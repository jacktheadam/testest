// One implementation, in `packages/transport-safety`. This path exists because
// scripts and tools already reach for it; it re-exports rather than restates.
module.exports = require("../../packages/transport-safety/src/text.cjs");
