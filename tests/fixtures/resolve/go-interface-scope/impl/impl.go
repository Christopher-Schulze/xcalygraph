package impl

type Disk struct{}

func (Disk) Read() string { return "disk" }

func (Disk) Shut() string { return "disk" }
