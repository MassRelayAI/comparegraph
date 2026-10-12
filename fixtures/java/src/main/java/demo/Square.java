package demo;

public class Square extends Base implements Shape {
    private double side = 2;

    public double area() {
        return side * side;
    }
}
