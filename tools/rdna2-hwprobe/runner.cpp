#include <hip/hip_runtime.h>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>
#define CK(x) do { hipError_t e = (x); if (e != hipSuccess) { fprintf(stderr, "%s:%d %s: %s\n", __FILE__, __LINE__, #x, hipGetErrorString(e)); return 1; } } while (0)
static std::vector<char> slurp(const std::string& p) { std::ifstream f(p, std::ios::binary); return std::vector<char>((std::istreambuf_iterator<char>(f)), {}); }
int main(int argc, char** argv) {
  std::string dir = argv[1]; const unsigned N = 65536;
  CK(hipInit(0));
  hipModule_t mod; CK(hipModuleLoad(&mod, (dir + "/k.co").c_str()));
  std::ifstream man(dir + "/manifest.txt"); std::string line; int runs = 0;
  while (std::getline(man, line)) {
    std::istringstream ss(line); std::string kn, inp, outp; size_t outsz; ss >> kn >> inp >> outsz >> outp;
    hipFunction_t fn; CK(hipModuleGetFunction(&fn, mod, kn.c_str()));
    auto in = slurp(dir + "/" + inp);
    void *din, *dout; CK(hipMalloc(&din, in.size())); CK(hipMalloc(&dout, outsz));
    CK(hipMemcpy(din, in.data(), in.size(), hipMemcpyHostToDevice));
    CK(hipMemset(dout, 0xAA, outsz));
    struct { void* a; void* b; unsigned long long n; } args = { din, dout, N };
    size_t asz = sizeof(args);
    void* cfg[] = { HIP_LAUNCH_PARAM_BUFFER_POINTER, &args, HIP_LAUNCH_PARAM_BUFFER_SIZE, &asz, HIP_LAUNCH_PARAM_END };
    CK(hipModuleLaunchKernel(fn, N / 256, 1, 1, 256, 1, 1, 0, nullptr, nullptr, cfg));
    CK(hipDeviceSynchronize());
    std::vector<char> out(outsz); CK(hipMemcpy(out.data(), dout, outsz, hipMemcpyDeviceToHost));
    std::ofstream(dir + "/" + outp, std::ios::binary).write(out.data(), out.size());
    CK(hipFree(din)); CK(hipFree(dout)); runs++;
  }
  printf("ran %d kernels\n", runs);
  return 0;
}
