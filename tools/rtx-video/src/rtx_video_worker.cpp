// SPDX-License-Identifier: MIT
// Independent RGB/YUV frame-filter process. NVIDIA SDK headers/runtime are external dependencies.
#define NOMINMAX
#include <windows.h>
#include <d3d11.h>
#include <dxgi1_2.h>
#include <wrl/client.h>
#include <nvsdk_ngx.h>
#include <nvsdk_ngx_helpers_vsr.h>
#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstdint>
#include <fcntl.h>
#include <filesystem>
#include <io.h>
#include <stdexcept>
#include <string>
#include <vector>
using Microsoft::WRL::ComPtr;
using Clock=std::chrono::steady_clock;

static void check(HRESULT hr,const char* what){if(FAILED(hr)){char b[160];sprintf_s(b,"%s: HRESULT 0x%08x",what,(unsigned)hr);throw std::runtime_error(b);}}
static void ngx(NVSDK_NGX_Result result,const char* what){if(NVSDK_NGX_FAILED(result)){char b[160];sprintf_s(b,"%s: NGX 0x%08x",what,(unsigned)result);throw std::runtime_error(b);}}
static uint8_t clip_byte(int v){return (uint8_t)std::clamp(v,0,255);}
static bool read(void* p,size_t n){return fread(p,1,n,stdin)==n;}
static void write(const void* p,size_t n){if(fwrite(p,1,n,stdout)!=n)throw std::runtime_error("output pipe closed");}
static void failure(const std::string& text){uint32_t n=(uint32_t)std::min<size_t>(text.size(),4096);fwrite("FAIL",1,4,stdout);fwrite(&n,4,1,stdout);fwrite(text.data(),1,n,stdout);fflush(stdout);fprintf(stderr,"rtx-video worker: %s\n",text.c_str());}

class Enhancer {
    ComPtr<ID3D11Device> device;
    ComPtr<ID3D11DeviceContext> context;
    ComPtr<ID3D11Texture2D> input,output,staging;
    NVSDK_NGX_Parameter* params=nullptr;
    NVSDK_NGX_Handle* feature=nullptr;
    bool initialized=false;
    uint32_t w,h,ow,oh,quality;
    std::vector<uint8_t> rgb,source,result;
public:
    Enhancer(uint32_t width,uint32_t height,uint32_t scale,uint32_t q):w(width),h(height),ow(w*scale),oh(h*scale),quality(q){
        if(w<2||h<2||w>4096||h>4096||(w|h)&1||scale<1||scale>4||ow>4096||oh>4096||quality>4)throw std::runtime_error("invalid frame dimensions/scale/quality");
        ComPtr<IDXGIFactory1> factory;check(CreateDXGIFactory1(IID_PPV_ARGS(&factory)),"DXGI factory");
        ComPtr<IDXGIAdapter1> adapter;
        for(UINT i=0;;++i){ComPtr<IDXGIAdapter1> a;if(factory->EnumAdapters1(i,&a)==DXGI_ERROR_NOT_FOUND)break;DXGI_ADAPTER_DESC1 desc{};a->GetDesc1(&desc);if(desc.VendorId==0x10de){adapter=a;break;}}
        if(!adapter)throw std::runtime_error("no NVIDIA adapter; keeping original video");
        check(D3D11CreateDevice(adapter.Get(),D3D_DRIVER_TYPE_UNKNOWN,nullptr,D3D11_CREATE_DEVICE_BGRA_SUPPORT,nullptr,0,D3D11_SDK_VERSION,&device,nullptr,&context),"D3D11 device");
        wchar_t name[32768];GetModuleFileNameW(nullptr,name,32768);auto dir=std::filesystem::path(name).parent_path();
        auto cache=dir/L"cache";if(auto player=_wgetenv(L"RAC_PLAYER_DIR"))cache=std::filesystem::path(player)/L"rtx-video-cache";
        std::filesystem::create_directories(cache);
        auto search=dir.wstring();const wchar_t* paths[]={search.c_str()};NVSDK_NGX_FeatureCommonInfo info{};info.PathListInfo.Path=paths;info.PathListInfo.Length=1;
        ngx(NVSDK_NGX_D3D11_Init(0,cache.c_str(),device.Get(),&info),"initialize RTX Video SDK");initialized=true;
        ngx(NVSDK_NGX_D3D11_GetCapabilityParameters(&params),"query VSR capability");
        int available=0;params->Get(NVSDK_NGX_Parameter_VSR_Available,&available);if(!available)throw std::runtime_error("RTX Video Super Resolution unavailable (GPU/driver/runtime)");
        NVSDK_NGX_Feature_Create_Params create{};ngx(NGX_D3D11_CREATE_VSR_EXT(context.Get(),&feature,params,&create),"create VSR feature");
        D3D11_TEXTURE2D_DESC desc{};desc.Width=w;desc.Height=h;desc.MipLevels=desc.ArraySize=desc.SampleDesc.Count=1;desc.Format=DXGI_FORMAT_R8G8B8A8_UNORM;desc.Usage=D3D11_USAGE_DEFAULT;desc.BindFlags=D3D11_BIND_SHADER_RESOURCE|D3D11_BIND_UNORDERED_ACCESS;
        check(device->CreateTexture2D(&desc,nullptr,&input),"input texture");desc.Width=ow;desc.Height=oh;check(device->CreateTexture2D(&desc,nullptr,&output),"output texture");
        desc.Usage=D3D11_USAGE_STAGING;desc.BindFlags=0;desc.CPUAccessFlags=D3D11_CPU_ACCESS_READ;check(device->CreateTexture2D(&desc,nullptr,&staging),"readback texture");
        rgb.resize(size_t(w)*h*4);source.resize(size_t(w)*h*3/2);result.resize(size_t(ow)*oh*3/2);
        fprintf(stderr,"rtx-video: NVIDIA SDK VSR ready, %ux%u -> %ux%u, quality %u\n",w,h,ow,oh,quality);
    }
    ~Enhancer(){if(feature)NVSDK_NGX_D3D11_ReleaseFeature(feature);if(params)NVSDK_NGX_D3D11_DestroyParameters(params);if(initialized)NVSDK_NGX_D3D11_Shutdown1(device.Get());}
    void loop(){
        write("OKAY",4);fflush(stdout);unsigned frames=0;double total=0,max_ms=0;
        while(read(source.data(),source.size())){
            auto begin=Clock::now();size_t plane=size_t(w)*h;auto cb=source.data()+plane;auto cr=cb+plane/4;
            for(uint32_t y=0;y<h;++y)for(uint32_t x=0;x<w;++x){size_t i=size_t(y)*w+x,c=size_t(y/2)*(w/2)+x/2;int Y=source[i]-16,U=cb[c]-128,V=cr[c]-128;rgb[i*4]=clip_byte((298*Y+409*V+128)>>8);rgb[i*4+1]=clip_byte((298*Y-100*U-208*V+128)>>8);rgb[i*4+2]=clip_byte((298*Y+516*U+128)>>8);rgb[i*4+3]=255;}
            context->UpdateSubresource(input.Get(),0,nullptr,rgb.data(),w*4,0);
            NVSDK_NGX_D3D11_VSR_Eval_Params eval{};eval.pInput=input.Get();eval.pOutput=output.Get();eval.InputSubrectSize={w,h};eval.OutputSubrectSize={ow,oh};eval.QualityLevel=(NVSDK_NGX_VSR_QualityLevel)quality;
            ngx(NGX_D3D11_EVALUATE_VSR_EXT(context.Get(),feature,params,&eval),"evaluate VSR");
            context->CopyResource(staging.Get(),output.Get());D3D11_MAPPED_SUBRESOURCE map{};check(context->Map(staging.Get(),0,D3D11_MAP_READ,0,&map),"read VSR result");
            auto dstY=result.data();auto dstU=dstY+size_t(ow)*oh;auto dstV=dstU+size_t(ow)*oh/4;
            for(uint32_t y=0;y<oh;y+=2)for(uint32_t x=0;x<ow;x+=2){int R=0,G=0,B=0;for(uint32_t dy=0;dy<2;++dy)for(uint32_t dx=0;dx<2;++dx){auto p=(const uint8_t*)map.pData+size_t(y+dy)*map.RowPitch+(x+dx)*4;int r=p[0],g=p[1],b=p[2];dstY[size_t(y+dy)*ow+x+dx]=clip_byte(16+((66*r+129*g+25*b+128)>>8));R+=r;G+=g;B+=b;}R=(R+2)/4;G=(G+2)/4;B=(B+2)/4;size_t c=size_t(y/2)*(ow/2)+x/2;dstU[c]=clip_byte(128+((-38*R-74*G+112*B+128)>>8));dstV[c]=clip_byte(128+((112*R-94*G-18*B+128)>>8));}
            context->Unmap(staging.Get(),0);auto us=(uint32_t)std::chrono::duration_cast<std::chrono::microseconds>(Clock::now()-begin).count();
            uint32_t header[]={ow,oh,us};write("FRAM",4);write(header,sizeof(header));write(result.data(),result.size());fflush(stdout);++frames;total+=us/1000.;max_ms=std::max(max_ms,us/1000.);
        }
        if(frames)fprintf(stderr,"rtx-video: %u frames, worker mean %.3f ms, max %.3f ms\n",frames,total/frames,max_ms);
    }
};

int main(){
    SetErrorMode(SEM_FAILCRITICALERRORS|SEM_NOGPFAULTERRORBOX);_setmode(_fileno(stdin),_O_BINARY);_setmode(_fileno(stdout),_O_BINARY);
    try{char magic[4];uint32_t cfg[4];if(!read(magic,4)||memcmp(magic,"RRV1",4)||!read(cfg,sizeof(cfg)))return 2;Enhancer e(cfg[0],cfg[1],cfg[2],cfg[3]);e.loop();return 0;}
    catch(const std::exception& e){failure(e.what());return 1;}
}
