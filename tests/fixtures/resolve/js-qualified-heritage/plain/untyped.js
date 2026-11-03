import * as defs from './defs.js';
class Named extends defs.Base {}
function build() {
  return class extends defs.Base {};
}
