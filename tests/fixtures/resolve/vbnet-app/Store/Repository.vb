Namespace App.Store

    ''' <summary>Reads rows by id.</summary>
    Public Interface IRepository
        Function Find(id As Integer) As Row
    End Interface

    Public Class Row
        Public Property Id As Integer
        Public Property Name As String
    End Class

    Public Class MemoryRepository
        Implements IRepository

        Public Function Find(id As Integer) As Row Implements IRepository.Find
            Return New Row()
        End Function
    End Class

End Namespace
