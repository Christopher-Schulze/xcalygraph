// Top-level overloads with signatures longer than the signature limit: the argument count
// still tells them apart.
public func render<T: CustomStringConvertible>(_ value: T, columns: Int = 80, indentation: Int = 2, separator: String = ", ", terminator: String = "\n") -> String {
  return "\(value)"
}

public func render<T: CustomStringConvertible, U: CustomStringConvertible>(_ value: T, root: U, header: String, columns: Int = 80, indentation: Int = 2, separator: String = ", ") -> String {
  return "\(root)\(value)"
}
