class Base {}
interface Marker {}
function build() {
  return class extends Base implements Marker {};
}
class Named extends Base implements Marker {}
abstract class AbstractNamed extends Base implements Marker {}
