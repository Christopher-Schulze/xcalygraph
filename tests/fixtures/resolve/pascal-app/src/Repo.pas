unit Repo;

interface

type
  TRepo = class
  public
    procedure Load(const Path: string);
    function Count: Integer;
  end;

implementation

procedure TRepo.Load(const Path: string);
begin
end;

function TRepo.Count: Integer;
begin
  Result := 0;
end;

end.
