import java.util.List;

public class Sample {
    private int total;

    static int bump(int n) {
        return n + 1;
    }

    public int add(int n) {
        total += bump(n);
        return total;
    }

    public static int run(List<Integer> xs) {
        Sample s = new Sample();
        return s.add(xs.size());
    }
}
