package oracle
func First() {}
func Second() {}
func Left() { First() }; func Right() { Second() }
func Multi() {
 First()
}
