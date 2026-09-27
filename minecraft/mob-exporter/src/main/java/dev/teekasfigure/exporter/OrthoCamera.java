package dev.teekasfigure.exporter;

import net.minecraft.client.MinecraftClient;
import org.joml.Matrix4f;

public final class OrthoCamera {
    public static boolean enabled=true;
    public static boolean active(){var status=WorldVideoPlayer.status;return status!=null && status.view()!=null;}
    public static float farPlane() {
        var status=WorldVideoPlayer.status;return status==null || status.view()==null?512:farPlane(status.view());
    }
    private static float farPlane(WorldVideoPlayer.View v) {
        var client=MinecraftClient.getInstance();
        var camera=client.gameRenderer.getCamera().getPos();
        var center=v.mode()==SceneTimeline.View.TOP
            ?new net.minecraft.util.math.Vec3d(v.x()+v.width()/2,v.baseY(),v.z()+v.height()/2)
            :new net.minecraft.util.math.Vec3d(v.x()+v.width()/2,v.baseY()+v.height()/2,v.z());
        return (float)Math.min(20_000_000,Math.max(512,camera.distanceTo(center)+v.topY()-v.baseY()+256));
    }
    public static Matrix4f projection(double fov) {
        var status=WorldVideoPlayer.status;if(status==null || status.view()==null)return null;var v=status.view();
        var window=MinecraftClient.getInstance().getWindow();float aspect=(float)window.getFramebufferWidth()/Math.max(1,window.getFramebufferHeight());
        if(!enabled)return new Matrix4f().perspective((float)Math.toRadians(fov),aspect,0.05f,farPlane(v));
        float half=(float)Math.max(v.height(),v.width()/aspect)*0.525f;
        return new Matrix4f().setOrtho(-half*aspect,half*aspect,-half,half,0.05f,farPlane(v));
    }
}
