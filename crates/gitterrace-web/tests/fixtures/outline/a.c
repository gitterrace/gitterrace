#include <stdio.h>
#define SQ(x) ((x)*(x))
struct point { int x; };
typedef struct point point_t;
enum color { RED };
static int area(struct point p) {
    return p.x;
}
int main(void) { return area((struct point){1}); }
