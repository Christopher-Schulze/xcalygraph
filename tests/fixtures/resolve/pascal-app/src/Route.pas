unit Route;

interface

uses Repo;

type
  IRoute<T> = interface
    procedure Run;
  end;

  TRoute<T: TRepo> = class(TInterfacedObject, IRoute<T>)
  public
    procedure Run;
  end;

implementation

procedure TRoute<T>.Run;
begin
end;

end.
