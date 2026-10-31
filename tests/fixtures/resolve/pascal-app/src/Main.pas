unit Main;

interface

uses Forms, Repo;

type
  TMainForm = class(TForm)
  private
    FRows: TRepo;
  public
    procedure OpenButtonClick(Sender: TObject);
    procedure Refresh;
  end;

implementation

procedure TMainForm.OpenButtonClick(Sender: TObject);
begin
  FRows.Load('rows.db');
  Refresh;
end;

procedure TMainForm.Refresh;
begin
  Caption := IntToStr(FRows.Count());
end;

end.
