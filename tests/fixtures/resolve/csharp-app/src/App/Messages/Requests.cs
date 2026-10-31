namespace App.Messages;

// Generic overloads by type parameter count share one name.
public interface IRequest { }

public interface IRequest<out TResponse> { }

public class Ping : IRequest<string> { }

public class Notify : IRequest { }
