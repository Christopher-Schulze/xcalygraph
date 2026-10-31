// CommonJS module object: members are defined as `app.x = function` and call each other
// through `this`; `exports.x` and `require(...).x` cross files; prototypes define methods.
var trim = require("./util").trim;
var app = exports = module.exports = {};

app.enabled = function enabled(setting) {
  return Boolean(setting);
};

app.render = function render(name) {
  if (this.enabled(name)) {
    return trim(name);
  }
  return "";
};

exports.compile = function compile(value) {
  return app.render(value);
};

function View(name) {
  this.name = name;
}

View.prototype.lookup = function lookup() {
  return exports.compile(this.name);
};
