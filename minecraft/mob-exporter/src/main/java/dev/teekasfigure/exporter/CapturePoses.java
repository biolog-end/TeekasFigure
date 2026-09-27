package dev.teekasfigure.exporter;
import net.minecraft.entity.LivingEntity;
import net.minecraft.entity.passive.BatEntity;
import net.minecraft.entity.passive.AxolotlEntity;
import net.minecraft.entity.passive.TadpoleEntity;
/** A reproducible flying pose gives bats visible wings in both capture and playback. */
public final class CapturePoses {
    public static String forCapture(LivingEntity entity,CaptureRenderer.View view){
        if(view==CaptureRenderer.View.FRONT)return entity instanceof BatEntity?"bat_flying":"";
        return entity instanceof BatEntity?"bat_top_v2":entity instanceof AxolotlEntity?"axolotl_top_v2":entity instanceof TadpoleEntity?"tadpole_top_v2":"";
    }
    public static boolean matches(LivingEntity entity,String pose){var info=Participant.metadata(entity);return info.owned() && info.pose().equals(pose);}
    public static void apply(LivingEntity entity,String pose){if(entity instanceof BatEntity bat){
        if(pose.equals("bat_flying")){bat.age=0;bat.setRoosting(false);bat.roostingAnimationState.stop();bat.flyingAnimationState.start(-5);}
        else if(pose.equals("bat_top_v2")){bat.age=0;bat.setRoosting(false);bat.roostingAnimationState.stop();bat.flyingAnimationState.stop();}
    }}
}
