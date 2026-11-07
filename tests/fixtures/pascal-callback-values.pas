unit Callbacks;
interface
implementation
procedure Handler(Value: Integer);
begin
end;
function Answer: Integer;
begin
  Result := 42;
end;
initialization
  RegisterHandler(Handler);
  Consume(Answer);
end.
