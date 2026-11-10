package a
trait Shape { def area: Double }
class Square(s: Double) extends Shape {
  def area: Double = s * s
}
object Main { def main(a: Array[String]): Unit = () }
