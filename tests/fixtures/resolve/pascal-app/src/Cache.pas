unit Cache;

interface

uses Repo;

type
  TCache = class;

  TCacheEntry = class
  public
    Owner: TCache;
  end;

  TCache = class(TRepo)
  public
    procedure Flush;
  end;

implementation

procedure TCache.Flush;
var
  Total: Integer;
begin
  Load('cache');
  Total := Count;
end;

end.
