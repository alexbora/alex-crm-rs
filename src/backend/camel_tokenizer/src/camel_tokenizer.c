#include <ctype.h>
#ifdef SQLITE_CORE
#include <sqlite3.h>
#else
#include <sqlite3ext.h>
SQLITE_EXTENSION_INIT1
#endif

#ifndef ALWAYS_INLINE
#if defined(__GNUC__) || defined(__clang__)
#define ALWAYS_INLINE inline __attribute__((always_inline))
#elif defined(_MSC_VER)
#define ALWAYS_INLINE __forceinline
#else
#define ALWAYS_INLINE inline
#endif
#endif

#include <string.h>

typedef struct CamelTokenizer {
  int dummy;
} CamelTokenizer;

// Stack-first token normalization buffer.
// Keep this explicit so fallback behavior is intentional and documented.
// Tokens larger than this use sqlite3_malloc for correctness (rare path).
#ifndef CAMEL_STACK_TOKEN_CAP
#define CAMEL_STACK_TOKEN_CAP 512
#endif

static int camelCreate(void *pUnused, const char **azArg, int nArg,
                       Fts5Tokenizer **ppOut) {
  CamelTokenizer *p = (CamelTokenizer *)sqlite3_malloc(sizeof(CamelTokenizer));
  if (!p)
    return SQLITE_NOMEM;
  *ppOut = (Fts5Tokenizer *)p;
  return SQLITE_OK;
}

static ALWAYS_INLINE void camelDelete(Fts5Tokenizer *pTok) {
  sqlite3_free(pTok);
}

static ALWAYS_INLINE int camelTokenize(Fts5Tokenizer *pTok, void *pCtx,
                                       int flags, const char *text, int len,
                                       int (*xToken)(void *, int, const char *,
                                                     int, int, int)) {
  int pos = 0;
  char buffer[CAMEL_STACK_TOKEN_CAP];
  (void)pTok;
  (void)flags;

  while (pos < len) {
    // 1. Skip non-alphanumeric
    while (pos < len && !isalnum((unsigned char)text[pos]))
      pos++;
    if (pos >= len)
      break;

    int start = pos;
    pos++;

    // 2. Scan for the end of the token
    while (pos < len) {
      unsigned char prev = (unsigned char)text[pos - 1];
      unsigned char curr = (unsigned char)text[pos];

      if (!isalnum(curr))
        break;

      // Split: lowerToUpper (camelCase) OR UpperUpperlower (ACRONYMLower)
      if (islower(prev) && isupper(curr))
        break;

      // Optional: Handle acronyms like JSONParser -> JSON, Parser
      if (pos + 1 < len) {
        if (isupper(prev) && isupper(curr) &&
            islower((unsigned char)text[pos + 1]))
          break;
      }

      pos++;
    }

    int tLen = pos - start;

    // 3. Normalization (CRITICAL for FTS5)
    // Convert to lowercase so that queries for 'camel' match 'Camel'
    char *pFinalToken;
    char *heapToken = NULL;

    // Stack path accepts tokens up to and including CAMEL_STACK_TOKEN_CAP bytes.
    // No NUL terminator is needed because xToken receives explicit length.
    if (tLen <= (int)sizeof(buffer)) {
      pFinalToken = buffer;
    } else {
      // Rare oversize-token path; keep correctness for arbitrary token lengths.
      heapToken = sqlite3_malloc(tLen);
      if (!heapToken)
        return SQLITE_NOMEM;
      pFinalToken = heapToken;
    }

    for (int i = 0; i < tLen; i++) {
      pFinalToken[i] = (char)tolower((unsigned char)text[start + i]);
    }
    int rc = xToken(pCtx, 0, pFinalToken, tLen, start, pos);
    if (heapToken)
      sqlite3_free(heapToken);
    if (rc != SQLITE_OK)
      return rc;
  }
  return SQLITE_OK;
}

#ifdef __cplusplus
extern "C"
#endif
#ifdef _WIN32
__declspec(dllexport)
#endif
int sqlite3_camel_init(sqlite3 *db, char **pzErrMsg,
                       const sqlite3_api_routines *pApi) {
#ifndef SQLITE_CORE
  SQLITE_EXTENSION_INIT2(pApi);
#else
  (void)pApi;
#endif
  fts5_api *ftsApi = NULL;
  sqlite3_stmt *pStmt = NULL;

  int rc = sqlite3_prepare_v2(db, "SELECT fts5(?)", -1, &pStmt, NULL);
  if (rc != SQLITE_OK)
    return rc;

  sqlite3_bind_pointer(pStmt, 1, (void *)&ftsApi, "fts5_api_ptr", NULL);
  sqlite3_step(pStmt);
  sqlite3_finalize(pStmt);

  if (!ftsApi) {
    *pzErrMsg = sqlite3_mprintf("FTS5 extension not found");
    return SQLITE_ERROR;
  }

  fts5_tokenizer camelTokenizer = {camelCreate, camelDelete, camelTokenize};
  return ftsApi->xCreateTokenizer(ftsApi, "camel", (void *)ftsApi,
                                  &camelTokenizer, NULL);
}
