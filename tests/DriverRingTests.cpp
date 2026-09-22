#include "../driver/ToasterMic.Driver/Source/Inc/ToasterRing.h"
#include <cassert>
#include <cstdio>
int main() {
    ToasterRing<16> ring;
    unsigned char a[8] = {1,2,3,4,5,6,7,8}, b[16]{};
    ring.Read(b, 8); assert(b[0] == 0 && ring.Size() == 0);
    assert(ring.Write(a, 8)); ring.Read(b, 4); assert(b[0] == 1 && b[3] == 4 && ring.Size() == 4);
    ring.Read(b, 8); assert(b[0] == 5 && b[3] == 8 && b[4] == 0);
    assert(!ring.Write(a, 3) && !ring.Write(a, 0) && !ring.Write(nullptr, 4));
    ring.Write(a, 8); ring.Write(a, 8); ring.Write(a, 8); assert(ring.Size() == 16);
    ring.Read(b, 16); assert(b[0] == 1 && b[8] == 1 && b[15] == 8);
    ring.Write(a, 8); ring.Reset(); ring.Read(b, 8); assert(b[0] == 0 && ring.Size() == 0);
    for (int i = 0; i < 10000; ++i) { ring.Write(a, 8); ring.Read(b, 4); assert(ring.Size() <= 16); }
    std::puts("PASS driver PCM queue: order, wrap, overrun, underrun, reset, invalid input, bounded growth");
}
