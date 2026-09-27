package dev.teekasfigure.verify;
import dev.teekasfigure.exporter.Localizations;
import dev.teekasfigure.exporter.DesktopIntegration;
import dev.teekasfigure.exporter.SceneTimeline;
import org.lwjgl.util.tinyfd.TinyFileDialogs;
import com.google.gson.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.atomic.*;
public final class UiRegressionCheck {
    private static int passed;
    private static void check(boolean value,String description){if(!value)throw new AssertionError(description);passed++;System.out.println("PASS "+description);}
    public static void main(String[] args)throws Exception{
        check(Boolean.getBoolean("java.awt.headless"),"test runs with Minecraft's AWT setting");
        check(Localizations.fallback("teekasfigure.player.start","ru_ru").equals("Запустить сцену"),"Russian labels load directly from classpath without a resource-pack loader");
        check(Localizations.fallback("teekasfigure.player.start","en_us").equals("Start scene"),"English labels load directly from classpath");
        check(Localizations.fallback("teekasfigure.player.start","fr_fr").equals("Start scene"),"unsupported languages have readable English fallback");
        check(TinyFileDialogs.tinyfd_getGlobalChar("tinyfd_version")!=null,"Minecraft's native dialog library loads with AWT disabled (no dialog is opened)");
        Path base=Files.createTempDirectory(Path.of("build"),"ui-check-");
        Path root=base.resolve("экспорт с пробелами [1]");Files.createDirectories(root);
        Path old=root.resolve("mobs_top_20260918_100000"),latest=root.resolve("mobs_front_20260918_110000");
        Files.createDirectory(old);Files.createDirectory(latest);Files.createDirectory(root.resolve("unrelated"));
        check(DesktopIntegration.latestExport(root).equals(latest),"reopened export menu finds the latest top/front export");
        Path empty=base.resolve("empty");Files.createDirectory(empty);
        check(DesktopIntegration.latestExport(empty).equals(empty),"folder action works before the first export");
        var command=DesktopIntegration.folderCommand(latest,"Windows 11","C:\\Windows");
        check(command.size()==2 && command.get(0).endsWith("explorer.exe") && command.get(1).equals(latest.toAbsolutePath().toString()),"Explorer gets a separate literal Unicode path argument");
        check(DesktopIntegration.normalizeScene(latest).equals(latest.toAbsolutePath().resolve("scene.json")),"pasted or dropped scene folder resolves to scene.json");
        var rootJson=JsonParser.parseString("{\"schema_version\":1,\"complete\":true,\"canvas_px\":[320,240],\"fps\":20,\"frame_count\":1,\"brushes\":[{}]}").getAsJsonObject();
        Files.writeString(latest.resolve("scene.json"),rootJson.toString());
        String row="[{\"brush_index\":0,\"entity_type\":\"minecraft:bat\",\"reference_size_blocks\":[2,2],\"depth_blocks\":1,\"pivot_blocks\":[0,0,0],\"yaw_offset_degrees\":180,\"pose\":\"bat_flying\"}]";
        Files.writeString(latest.resolve("minecraft_mapping.json"),row);
        Files.writeString(latest.resolve("frames.jsonl"),"{\"frame_index\":0,\"time_seconds\":0,\"shapes\":[{\"id\":1,\"brush_index\":0,\"original_colors\":true,\"opacity\":1,\"hue_turns\":0,\"saturation\":1,\"brightness\":1,\"size_px\":[32,32],\"center_px\":[160,120],\"rotation_radians\":0}]}\n");
        try(var scene=new SceneTimeline(latest.resolve("scene.json"),256,new AtomicBoolean(),new AtomicInteger())){
            check(scene.view==SceneTimeline.View.TOP,"legacy mappings without an explicit view remain top-view scenes");
            check(scene.brushes[0].pose().equals("bat_flying") && scene.advanceTo(0).shapes().size()==1,"flying-bat pose survives scene decoding");
            scene.cancel();try{scene.advanceTo(1);check(false,"cancelled timeline rejects further reads");}catch(java.io.IOException ignored){}
        }
        Files.writeString(latest.resolve("minecraft_mapping.json"),row.replace("\"pose\":\"bat_flying\"","\"pose\":\"bat_flying\",\"view\":\"front\""));
        try(var scene=new SceneTimeline(latest.resolve("scene.json"),256,new AtomicBoolean(),new AtomicInteger())){
            check(scene.view==SceneTimeline.View.FRONT,"front-view mapping selects the forward-facing stage and camera");
        }
        // Remove only this test's newly created directory, never any game directory.
        try(var files=Files.walk(base)){for(var file:files.sorted(Comparator.reverseOrder()).toList())Files.delete(file);}
        System.out.println("UI regression checks passed: "+passed);
    }
}
