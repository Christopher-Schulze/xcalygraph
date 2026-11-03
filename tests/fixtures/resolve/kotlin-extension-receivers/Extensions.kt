package app

fun String.shout(): String = uppercase()

fun Int.shout(): String = toString()

fun List<String>.only(): String = first()

fun use(text: String, count: Int, items: List<String>): String {
    val a = text.shout()
    val b = count.shout()
    val c = items.only()
    return a + b + c
}
