#include <cinttypes>
#include <cstdint>
#include <iostream>

#ifdef DO_WEVAL
#ifndef __wasi__
#error "DO_WEVAL is only supported on WASI"
#endif
#endif

#ifdef DO_WEVAL
#include "weval.h"
#include "wizer.h"
#endif

#define ALWAYS_INLINE inline __attribute__((always_inline))
#define NEVER_INLINE __attribute__((noinline))

typedef int64_t word;
typedef uint64_t uword;
typedef uword Object;

#define FOR_EACH_INSTRUCTION(V)                                                \
  V(LOAD_IMMEDIATE)                                                            \
  V(STORE_LOCAL)                                                               \
  V(LOAD_LOCAL)                                                                \
  V(PRINT)                                                                     \
  V(PRINTI)                                                                    \
  V(JMPNZ)                                                                     \
  V(INC)                                                                       \
  V(DEC)                                                                       \
  V(ADD)                                                                       \
  V(HALT)

typedef enum {
#define ENUM(name) name,
  FOR_EACH_INSTRUCTION(ENUM)
#undef ENUM
} Instruction;

template <bool IsSpecialized>
static NEVER_INLINE Object Execute(uword *program) {
  Object accumulator = 0;
  Object locals[256] = {0};
  Object stack[256] = {0};
  word sp = 0;
  Object tmp;
#define DO_PUSH(x) (stack[sp++] = (x))
#define DO_POP() (tmp = stack[--sp], stack[sp] = (Object)0, tmp)
#ifdef DO_WEVAL
#define LOCAL_AT(idx) (IsSpecialized ? weval_read_reg(idx) : locals[idx])
#define LOCAL_AT_PUT(idx, val)                                                 \
  if (IsSpecialized) {                                                         \
    weval_write_reg(idx, val);                                                 \
  } else {                                                                     \
    locals[idx] = val;                                                         \
  }
#else
#define LOCAL_AT(idx) (locals[idx])
#define LOCAL_AT_PUT(idx, val) (locals[idx] = val)
#endif

#ifdef DO_WEVAL
  weval::push_context(0);
#endif
  uint32_t pc = 0;

  while (true) {
#ifdef DO_WEVAL
    weval_assert_const32(pc, __LINE__);
#endif
    Instruction op = (Instruction)program[pc++];
    switch (op) {
    case LOAD_IMMEDIATE: {
      uword value = program[pc++];
      accumulator = (Object)value;
      break;
    }
    case STORE_LOCAL: {
      uword idx = program[pc++];
      LOCAL_AT_PUT(idx, accumulator);
      break;
    }
    case LOAD_LOCAL: {
      uword idx = program[pc++];
      accumulator = LOCAL_AT(idx);
      break;
    }
    case PRINT: {
      const char *msg = (const char *)program[pc++];
      printf("%s", msg);
      break;
    }
    case PRINTI: {
      printf("%" PRIu64, accumulator);
      break;
    }
    case HALT: {
      return accumulator;
    }
    case JMPNZ: {
      uword offset = program[pc++];
      if (accumulator != 0) {
        pc = offset;
      }
      break;
    }
    case INC: {
      accumulator++;
      break;
    }
    case DEC: {
      accumulator--;
      break;
    }
    case ADD: {
      uword idx1 = program[pc++];
      uword idx2 = program[pc++];
      accumulator = LOCAL_AT(idx1) + LOCAL_AT(idx2);
      break;
    }
    default: {
      fprintf(stderr, "Unknown opcode: %d\n", op);
      return 0;
    }
    }
#ifdef DO_WEVAL
    weval::update_context(pc);
#endif
  }
#ifdef DO_WEVAL
  weval::pop_context();
#endif
}

Object (*ExecuteSpecialized)(uword *) = 0;

enum {
  result,
  loopc,
  goal = 100000000,
};
// clang-format off
uword program[] = {
  LOAD_IMMEDIATE, 0,
  STORE_LOCAL, result,
  LOAD_IMMEDIATE, goal,
  STORE_LOCAL, loopc,

  ADD, result, loopc,
  STORE_LOCAL, result,
  LOAD_LOCAL, loopc,
  DEC,
  STORE_LOCAL, loopc,
  JMPNZ, 8,

  PRINT, (uword)"Result: ",
  LOAD_LOCAL, result,
  PRINTI,
  PRINT, (uword)"\n",
  HALT,
};
// clang-format on

#ifdef DO_WEVAL
void init() {
  uword result = 0;
  uword loopc = 1;
  weval_req_t *req = weval_build_request((weval_func_t)&Execute<true>,
                                         (weval_func_t *)&ExecuteSpecialized);
  weval_append_memory_arg(req, program, sizeof program);
  weval_request(req);
}

WIZER_INIT(init);
WEVAL_DEFINE_GLOBALS();
#endif

int main(int argc, char **argv) {
  if (ExecuteSpecialized) {
    ExecuteSpecialized(nullptr);
  } else {
    Execute<false>(program);
  }
}
