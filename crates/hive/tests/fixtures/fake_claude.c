/* A stand-in process for the tests that need one named `claude` (or `node`): never the real
 * Claude Code. The tests build it with `cc`, since macOS 26 kills a copied system binary run
 * from another folder. It opens every argument ending in `.jsonl` (a session log, as Claude
 * Code holds one open), then sleeps 30 s. */
#include <fcntl.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char **argv) {
    for (int i = 1; i < argc; i++) {
        size_t n = strlen(argv[i]);
        if (n > 6 && strcmp(argv[i] + n - 6, ".jsonl") == 0) {
            open(argv[i], O_RDONLY);
        }
    }
    sleep(30);
    return 0;
}
