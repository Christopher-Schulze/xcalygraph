package fixture

import "github.com/spf13/pflag"

type Command struct{}
type LocalFlagSet struct{}

func (cmd *Command) NonInheritedFlags() *pflag.FlagSet { return nil }
func (cmd *Command) HasFlags() bool                    { return false }
func (cmd *Command) LocalFlags() *LocalFlagSet         { return nil }
func (flags *LocalFlagSet) Count() int                 { return 0 }

func hasExternalFlags(cmd *Command) bool {
	flags := cmd.NonInheritedFlags()
	return flags.HasFlags()
}

func hasExternalFlagsDirect(cmd *Command) bool {
	return cmd.NonInheritedFlags().HasFlags()
}

func hasLocalFlags(cmd *Command) int {
	flags := cmd.LocalFlags()
	return flags.Count()
}
