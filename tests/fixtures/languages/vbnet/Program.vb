Imports App.Store
Imports Repo = App.Store.IRepository

Namespace App.Web

    Public Module Program
        Public Sub Main()
            Dim repository As Repo = New MemoryRepository()
            Dim row As Row = repository.Find(1)
            Dim url = Environment.GetEnvironmentVariable("APP_URL")
            Report(row)
        End Sub

        Private Sub Report(row As Row)
            Console.WriteLine(row.Name)
        End Sub
    End Module

End Namespace
