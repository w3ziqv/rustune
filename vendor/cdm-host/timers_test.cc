#include "timers.h"
#include <cassert>
int main() {
  CdmTimers timers;
  auto start = CdmTimers::Clock::time_point(std::chrono::seconds(1));
  int marker = 0, callbacks = 0;
  assert(timers.add(20, &marker, start));
  auto callback = [&](void* context) { assert(context == &marker); ++callbacks; };
  timers.pump(callback, start);
  timers.pump(callback, start + std::chrono::milliseconds(19));
  assert(callbacks == 0);
  timers.pump(callback, start + std::chrono::milliseconds(20));
  assert(callbacks == 1);
  timers.pump(callback, start + std::chrono::seconds(1));
  assert(callbacks == 1);
  for (int i = 0; i < 128; ++i) assert(timers.add(100, &marker, start));
  assert(!timers.add(100, &marker, start));
  CdmTimers reentrant;
  assert(reentrant.add(0, &marker, start));
  for (int i = 0; i < 127; ++i) assert(reentrant.add(100, &marker, start));
  reentrant.pump([&](void*) {
    assert(reentrant.add(1, &marker, start));
    assert(!reentrant.add(1, &marker, start));
  }, start);
  int future_callbacks = 0;
  reentrant.pump([&](void*) { ++future_callbacks; }, start + std::chrono::seconds(1));
  assert(future_callbacks == 128);
}
