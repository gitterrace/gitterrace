namespace geo {
class Shape {
public:
    virtual double area() const;
    void draw() { }
};
}
double geo::Shape::area() const { return 0; }
int main() { return 0; }
