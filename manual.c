#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>

int main() {
  NO_UNROLL uint64_t goal = 100000000;
  NO_UNROLL uint64_t result = 0;
  NO_UNROLL uint64_t loopc = goal;
begin:
  result += loopc;
  loopc--;
  if (loopc != 0)
    goto begin;
  printf("Result: %" PRIu64 "\n", result);
  return 0;
}
