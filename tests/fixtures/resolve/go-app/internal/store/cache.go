package store

func warm() {
	st, _ := NewStore("x")
	st.Get(1)
	count(st)
}
