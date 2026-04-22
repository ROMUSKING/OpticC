struct FuncDestructor;
struct sqlite3_context;
struct sqlite3_value;
struct FuncDef {
  short nArg;
  unsigned int funcFlags;
  void *pUserData;
  struct FuncDef *pNext;
  void (*xSFunc)(struct sqlite3_context*,int,struct sqlite3_value**);
  void (*xFinalize)(struct sqlite3_context*);
  void (*xValue)(struct sqlite3_context*);
  void (*xInverse)(struct sqlite3_context*,int,struct sqlite3_value**);
  const char *zName;
  union {
    struct FuncDef *pHash;
    struct FuncDestructor *pDestructor;
  } u;
};
void juliandayFunc(struct sqlite3_context*,int,struct sqlite3_value**) {}
int sqlite3Config = 0;
void test() {
  static struct FuncDef aDateTimeFuncs[] = {
    { -1, 1024|2048, &sqlite3Config, 0, juliandayFunc, 0, 0, 0, "julianday", {0} },
  };
}
