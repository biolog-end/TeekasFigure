package dev.teekasfigure.exporter;
import org.lwjgl.system.MemoryStack;
import org.lwjgl.util.tinyfd.TinyFileDialogs;
import java.io.IOException;
import java.nio.file.*;
import java.util.*;
/** Uses Minecraft's native dialogs, independently of java.awt.headless. */
public final class DesktopIntegration {
    public static Path chooseScene(String title,Path initial) {
        synchronized(DesktopIntegration.class) {
            try(var stack=MemoryStack.stackPush()) {
                TinyFileDialogs.tinyfd_setGlobalInt("tinyfd_winUtf8",1);
                var patterns=stack.mallocPointer(1);patterns.put(0,stack.UTF8("*.json"));
                String chosen=TinyFileDialogs.tinyfd_openFileDialog(title,initial.toAbsolutePath().toString(),patterns,"scene.json",false);
                return chosen==null?null:normalizeScene(Path.of(chosen));
            }
        }
    }
    public static Path normalizeScene(Path path){Path result=path.toAbsolutePath().normalize();return Files.isDirectory(result)?result.resolve("scene.json"):result;}
    public static List<String> folderCommand(Path folder,String os,String windowsRoot) {
        String path=folder.toAbsolutePath().normalize().toString();os=os.toLowerCase(Locale.ROOT);
        if(os.startsWith("windows"))return List.of(windowsRoot==null?"explorer.exe":Path.of(windowsRoot,"explorer.exe").toString(),path);
        if(os.startsWith("mac"))return List.of("open",path);
        return List.of("xdg-open",path);
    }
    public static void openFolder(Path folder) throws IOException {
        if(!Files.isDirectory(folder))throw new IOException("Folder does not exist: "+folder);
        new ProcessBuilder(folderCommand(folder,System.getProperty("os.name"),System.getenv("SystemRoot")))
            .redirectOutput(ProcessBuilder.Redirect.DISCARD).redirectError(ProcessBuilder.Redirect.DISCARD).start();
    }
    public static Path latestExport(Path root) throws IOException {
        if(!Files.isDirectory(root))return root;
        try(var paths=Files.list(root)) {
            return paths.filter(Files::isDirectory).filter(p -> exportStamp(p)!=null)
                .max(Comparator.comparing(DesktopIntegration::exportStamp)).orElse(root);
        }
    }
    private static String exportStamp(Path path) {
        String name=path.getFileName().toString();
        if(name.startsWith("mobs_top_"))return name.substring("mobs_top_".length());
        if(name.startsWith("mobs_front_"))return name.substring("mobs_front_".length());
        return null;
    }
}
