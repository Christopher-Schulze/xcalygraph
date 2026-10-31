unit Store;

interface

uses SysUtils, Repo;

type
  TStore = class(TBase, IStore)
  private
    FRows: TRepo;
  public
    procedure Open(const Path: string);
    function Count: Integer;
  end;

implementation

procedure TStore.Open(const Path: string);
var Url: string;
begin
  Url := GetEnvironmentVariable('STORE_URL');
  FRows.Load(Path);
  Helper(Path);
end;

function TStore.Count: Integer;
begin
  Result := FRows.Count;
end;

end.
