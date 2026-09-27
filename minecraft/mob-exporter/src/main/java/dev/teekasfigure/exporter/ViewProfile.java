package dev.teekasfigure.exporter;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.option.*;
/** Temporary viewing options. Never writes the options file and restores on exit. */
public final class ViewProfile {
    private record Saved(GameOptions options,CloudRenderMode clouds,ParticlesMode particles,boolean shadows,boolean bob,boolean hud,int view,int simulation,int fps) {}
    private static Saved saved;
    public static void tick(MinecraftClient client) {
        boolean active=client.world!=null && OrthoCamera.active();
        if(!active){restore();return;}
        if(saved!=null)return;
        var o=client.options;
        saved=new Saved(o,o.getCloudRenderMode().getValue(),o.getParticles().getValue(),o.getEntityShadows().getValue(),o.getBobView().getValue(),o.hudHidden,o.getViewDistance().getValue(),o.getSimulationDistance().getValue(),o.getMaxFps().getValue());
        o.getCloudRenderMode().setValue(CloudRenderMode.OFF);o.getParticles().setValue(ParticlesMode.MINIMAL);
        o.getEntityShadows().setValue(false);o.getBobView().setValue(false);o.hudHidden=true;
        o.getViewDistance().setValue(Math.min(saved.view(),2));o.getSimulationDistance().setValue(Math.min(saved.simulation(),5));o.getMaxFps().setValue(Math.min(saved.fps(),60));
    }
    public static void restore() {
        var s=saved;if(s==null)return;saved=null;var o=s.options();
        o.getCloudRenderMode().setValue(s.clouds());o.getParticles().setValue(s.particles());o.getEntityShadows().setValue(s.shadows());o.getBobView().setValue(s.bob());o.hudHidden=s.hud();
        o.getViewDistance().setValue(s.view());o.getSimulationDistance().setValue(s.simulation());o.getMaxFps().setValue(s.fps());
    }
}
