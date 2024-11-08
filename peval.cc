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
  V(STORE_REGISTER)                                                               \
  V(LOAD_REGISTER)                                                                \
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
  Object registers[256] = {0};
#if defined(DO_WEVAL) && defined(SPECIALIZE_REGISTERS)
#define REGISTER_AT(idx) weval_read_reg(idx)
#define REGISTER_AT_PUT(idx, val) weval_write_reg(idx, val)
#else
#define REGISTER_AT(idx) (registers[idx])
#define REGISTER_AT_PUT(idx, val) (registers[idx] = val)
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
    case STORE_REGISTER: {
      uword idx = program[pc++];
      REGISTER_AT_PUT(idx, accumulator);
      break;
    }
    case LOAD_REGISTER: {
      uword idx = program[pc++];
      accumulator = REGISTER_AT(idx);
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
      uword address = program[pc++];
      if (accumulator != 0) {
        pc = address;
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
      accumulator = REGISTER_AT(idx1) + REGISTER_AT(idx2);
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
  STORE_REGISTER, result,
  LOAD_IMMEDIATE, goal,
  STORE_REGISTER, loopc,

  ADD, result, loopc,
  STORE_REGISTER, result,
  LOAD_REGISTER, loopc,
  DEC,
  STORE_REGISTER, loopc,
  JMPNZ, 8,

  PRINT, (uword)"Result: ",
  LOAD_REGISTER, result,
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
