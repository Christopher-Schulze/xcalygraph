const { join } = require("path");
const util = require("./util");

function format(text) {
  return util.trim(join(text));
}

module.exports = { format };
