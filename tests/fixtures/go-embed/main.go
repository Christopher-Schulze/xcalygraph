package embedfixture

import "embed"

//go:embed data.txt "dir/space name.txt" `dir/raw.txt`
var Content embed.FS

var (
	//go:embed dir/sub/*
	Group embed.FS
)
