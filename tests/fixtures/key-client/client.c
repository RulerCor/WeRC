#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

/* 自有合成目标，不连接微信。只持有已知测试结构，不输出地址或密钥。 */
static const unsigned char marker[] = "com.Tencent.WCDB.Config.Cipher";
static unsigned char node[0x50], config[0xb0], blob[99];
static const unsigned char mask[32] = {
  0xd2,0xc7,0x44,0x24,0x58,0x02,0,0,0,0x48,0x89,0x44,0x24,0x50,0x48,0x8b,
  0x45,0,0x48,0x84,0x4c,0x24,0x48,0x48,0x89,0x44,0x25,0x40,0x48,0x58,0x4c,0x24
};
static void put64(unsigned char *at, uint64_t value) { memcpy(at,&value,8); }
int main(int argc, char **argv) {
  char input[32]; size_t i;
  if (argc != 2 || strcmp(argv[1],"--synthetic-memory-target") != 0) return 2;
  blob[0]='x'; blob[1]='\'';
  for(i=0;i<32;i++) { blob[2+i*2]='5'; blob[3+i*2]='a'; }
  for(i=0;i<16;i++) { blob[66+i*2]='3'; blob[67+i*2]='5'; }
  blob[98]='\'';
  for(i=0;i<sizeof(blob);i++) blob[i]^=mask[i%32];
  put64(node+0x10,(uint64_t)(uintptr_t)marker);
  put64(node+0x18,sizeof(marker)-1);
  put64(node+0x28,(uint64_t)(uintptr_t)config);
  put64(config+0x90,(uint64_t)(uintptr_t)blob);
  put64(config+0x98,sizeof(blob));
  puts("SYNTHETIC_READY"); fflush(stdout);
  if (!fgets(input,sizeof(input),stdin)) return 3;
  SecureZeroMemory(blob,sizeof(blob));
  SecureZeroMemory(config,sizeof(config));
  SecureZeroMemory(node,sizeof(node));
  return 0;
}
