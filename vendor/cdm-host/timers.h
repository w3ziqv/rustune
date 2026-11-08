#pragma once
#include <chrono>
#include <cstdint>
#include <vector>

// Deadlines are monotonic; the CDM's requested delays are never accelerated.
class CdmTimers {
 public:
  using Clock = std::chrono::steady_clock;
  bool add(int64_t delay_ms, void* context, Clock::time_point now = Clock::now()) {
    if (pending_.size() >= 128) return false;
    if (delay_ms < 0) delay_ms = 0;
    auto remaining = std::chrono::duration_cast<std::chrono::milliseconds>(Clock::time_point::max() - now).count();
    if (delay_ms > remaining) return false;
    pending_.push_back({now + std::chrono::milliseconds(delay_ms), context});
    return true;
  }
  template<class Callback>
  void pump(Callback callback, Clock::time_point now = Clock::now()) {
    auto scheduled = std::move(pending_);
    pending_.clear();
    std::vector<void*> due;
    due.reserve(scheduled.size());
    for (const auto& timer : scheduled) {
      if (timer.deadline <= now) due.push_back(timer.context);
      else pending_.push_back(timer);
    }
    // Reserve all surviving timers before callbacks can schedule more work.
    for (void* context : due) callback(context);
  }
 private:
  struct Timer { Clock::time_point deadline; void* context; };
  std::vector<Timer> pending_;
};
