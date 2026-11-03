import * as defs from './defs';
class Named extends defs.Base {}
function build() {
  return class extends defs.Base {};
}
