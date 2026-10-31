#include "code/util.dm"
/obj/item/store
	var/rows = 0
	proc/open(path)
		helper(path)
		src.count()
	proc/count()
		return rows
/obj/item/store/big/open(path)
	..()
/proc/helper(x)
	return x
