package impl

import "testing"

// fakeReader is a test type: it must not count as an implementation of production interfaces.
type fakeReader struct{}

func (fakeReader) Read() string { return "fake" }

func TestDisk(t *testing.T) {
	var disk Disk
	var fake fakeReader
	_ = disk.Read()
	_ = fake.Read()
}
