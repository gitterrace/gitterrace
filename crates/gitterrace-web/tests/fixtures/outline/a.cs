namespace Shop {
    public interface IItem { int Price(); }
    public class Cart {
        public int Total() { return 0; }
    }
    public struct Pt { public int X; }
    enum Size { S }
}
