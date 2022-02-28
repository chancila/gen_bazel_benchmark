#![feature(async_closure)]
#![feature(int_log)]

use anyhow::private::format_err;
use anyhow::{format_err, Result};
use clap::Parser;
use futures::{stream, StreamExt};
use itertools::Itertools;
use std::fmt::Display;
use std::future::Future;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use tokio::sync::watch;
use tokio_stream::iter;

/// Generate a bazel benchmarking workspace. You can tweak various parameters to configure the
/// topology of the build graph.
///
/// Generally the amount of targets generated will be targets_per_level^height
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Directory to write the output to, existing content will be wiped
    #[clap(long)]
    output: PathBuf,

    /// Height of the build graph
    #[clap(long)]
    height: u32,

    /// The amount of targets to generate per level, each
    #[clap(long)]
    targets_per_level: u64,

    #[clap(long)]
    files_per_target: u64,
}

async fn emit_build_file(
    node_id: u64,
    targets_per_level: u64,
    files_per_target: u64,
    root_dir: PathBuf,
    max_depth: u64,
) {
    tokio::spawn(async move {
        if node_id == 0 {
            handle_root(targets_per_level, &root_dir);
        } else {
            let id = ID::new(node_id, targets_per_level, max_depth);
            handle_node(&id, files_per_target, &root_dir);
        }
    })
    .await
    .unwrap();
}

const ALL_FRAMEWORKS: [&str; 135] = [
    "ARKit",
    "AVFAudio",
    "AVFoundation",
    "AVKit",
    "Accelerate",
    "Accessibility",
    "Accounts",
    "AdServices",
    "AdSupport",
    "AddressBook",
    "AddressBookUI",
    "AppClip",
    "AppTrackingTransparency",
    "AssetsLibrary",
    "AudioToolbox",
    "AudioUnit",
    "AuthenticationServices",
    "AutomaticAssessmentConfiguration",
    "BackgroundTasks",
    "BusinessChat",
    "CFNetwork",
    "CallKit",
    "CarPlay",
    "ClassKit",
    "ClockKit",
    "CloudKit",
    "Contacts",
    "ContactsUI",
    "CoreAudio",
    "CoreAudioKit",
    "CoreAudioTypes",
    "CoreBluetooth",
    "CoreData",
    "CoreFoundation",
    "CoreGraphics",
    "CoreHaptics",
    "CoreImage",
    "CoreLocation",
    "CoreLocationUI",
    "CoreMIDI",
    "CoreML",
    "CoreMedia",
    "CoreMotion",
    "CoreNFC",
    "CoreServices",
    "CoreSpotlight",
    "CoreTelephony",
    "CoreText",
    "CoreVideo",
    "DataDetection",
    "DeviceCheck",
    "EventKit",
    "EventKitUI",
    "ExposureNotification",
    "ExternalAccessory",
    "FileProvider",
    "FileProviderUI",
    "Foundation",
    "GLKit",
    "GSS",
    "GameController",
    "GameKit",
    "GameplayKit",
    "GroupActivities",
    "HealthKit",
    "HealthKitUI",
    "HomeKit",
    "IOSurface",
    "IdentityLookup",
    "IdentityLookupUI",
    "ImageCaptureCore",
    "ImageIO",
    "Intents",
    "IntentsUI",
    "JavaScriptCore",
    "LinkPresentation",
    "LocalAuthentication",
    "MapKit",
    "MediaAccessibility",
    "MediaPlayer",
    "MediaToolbox",
    "MessageUI",
    "Messages",
    "Metal",
    "MetalKit",
    "MetalPerformanceShaders",
    "MetalPerformanceShadersGraph",
    "MetricKit",
    "MobileCoreServices",
    "ModelIO",
    "MultipeerConnectivity",
    "NaturalLanguage",
    "NearbyInteraction",
    "Network",
    "NetworkExtension",
    "NewsstandKit",
    "NotificationCenter",
    "OSLog",
    "OpenAL",
    "OpenGLES",
    "PDFKit",
    "PHASE",
    "PassKit",
    "PencilKit",
    "Photos",
    "PhotosUI",
    "PushKit",
    "QuartzCore",
    "QuickLook",
    "QuickLookThumbnailing",
    "ReplayKit",
    "SafariServices",
    "SceneKit",
    "ScreenTime",
    "Security",
    "SensorKit",
    "ShazamKit",
    "Social",
    "SoundAnalysis",
    "Speech",
    "SpriteKit",
    "StoreKit",
    "SwiftUI",
    "SystemConfiguration",
    "UIKit",
    "UniformTypeIdentifiers",
    "UserNotifications",
    "UserNotificationsUI",
    "VideoToolbox",
    "Vision",
    "VisionKit",
    "WatchConnectivity",
    "WebKit",
    "WidgetKit",
    "iAd",
];

fn handle_root(targets_per_level: u64, root_dir: &Path) {
    let mut file = std::fs::File::create(root_dir.join("BUILD.bazel")).unwrap();
    let deps: String = (1..=targets_per_level)
        .map(|i| format!("\"//pkg_1/lib_{}\"", i))
        .intersperse(", ".to_string())
        .collect();

    writeln!(
        file,
        r#"load("@build_bazel_rules_ios//rules:app.bzl", "ios_application")
ios_application(
    name = "root",
    bundle_id = "com.bazel.benchmark",
    families = [
        "iphone",
        "ipad",
    ],
    srcs = ["main.m"],
    minimum_os_version = "15.0",
    deps = [{}],
)"#,
        deps
    )
    .unwrap();
}

#[derive(Clone)]
struct ID {
    id: u64,
    parents: Vec<ID>,
    package_relative_index: u64,
    targets_per_level: u64,
    max_depth: u64,
}

impl ID {
    fn new(id: u64, targets_per_level: u64, max_depth: u64) -> Self {
        let mut parents = vec![];
        let mut parent_id = id;

        if id != 0 {
            loop {
                parent_id = (parent_id - 1) / targets_per_level as u64;

                parents.push(ID::new(parent_id, targets_per_level, max_depth));

                if parent_id == 0 {
                    break;
                }
            }
        }

        let package_relative_index = if id > 0 {
            1 + id - num_nodes_in_ntree(targets_per_level, parents.len() as u32 - 1)
        } else {
            0
        };

        ID {
            id,
            parents,
            package_relative_index,
            targets_per_level,
            max_depth,
        }
    }

    fn build_file(&self) -> PathBuf {
        self.lib_path().join("BUILD.bazel")
    }

    fn package_path(&self) -> PathBuf {
        let res: String = (1..=self.parents.len())
            .map(|i| format!("pkg_{}", i))
            .intersperse("/".to_string())
            .collect();

        PathBuf::from(res)
    }

    fn lib_path(&self) -> PathBuf {
        self.package_path()
            .join(format!("lib_{}", self.package_relative_index))
    }

    fn lib_name(&self) -> String {
        let res: String = (1..=self.parents.len())
            .map(|i| format!("Pkg{}", i))
            .intersperse("_".to_string())
            .collect();

        format!("{}_Lib{}", res, self.package_relative_index)
    }

    fn children(&self) -> Vec<ID> {
        if self.parents.len() >= self.max_depth as usize {
            return vec![];
        }

        let mut result = vec![];

        let mut parents = self.parents.clone();
        parents.push(self.clone());

        for i in 0..self.targets_per_level {
            result.push(ID {
                id: self.id * self.targets_per_level + i,
                parents: parents.clone(),
                package_relative_index: self.targets_per_level * (self.package_relative_index - 1)
                    + i
                    + 1,
                targets_per_level: self.targets_per_level,
                max_depth: self.max_depth,
            })
        }

        result
    }
}

impl Display for ID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.lib_path())
    }
}

fn handle_node(node: &ID, files_per_target: u64, root_dir: &Path) {
    println!("handling {}", node);
    let lib_dir = root_dir.join(node.lib_path());
    std::fs::create_dir_all(&lib_dir).unwrap();

    let mut f = std::fs::File::create(&lib_dir.join("BUILD.bazel")).unwrap();

    writeln!(
        f,
        r#"load("@build_bazel_rules_ios//rules:framework.bzl", "apple_framework")"#
    )
    .unwrap();
    let deps_snippet: String = node
        .children()
        .iter()
        .map(|node| format!(r#""//{}""#, node.lib_path().to_str().unwrap()))
        .intersperse(", ".to_string())
        .collect();

    let srcs = (1..=files_per_target)
        .into_iter()
        .flat_map(|i| {
            vec![
                format!(r#""{}_Hdr{}.h""#, node.lib_name(), i),
                format!(r#""{}_Src{}.m""#, node.lib_name(), i),
            ]
        })
        .intersperse(", ".to_string());

    let srcs_snippet = "";
    writeln!(
        f,
        r#"apple_framework(name = "lib_{}",
    module_name = "{}",
    srcs = [
        {}
    ],
    deps = [
{}
    ],
    visibility = ["//visibility:public"])"#,
        node.package_relative_index,
        node.lib_name(),
        srcs.collect::<String>(),
        deps_snippet,
    )
    .unwrap();

    for i in 1..=files_per_target {
        write_objc_files(&lib_dir, node, files_per_target);
    }
}

fn write_objc_files(lib_dir: &Path, node: &ID, files_per_target: u64) {
    for i in 1..=files_per_target {
        let mut hdr_file = BufWriter::new(
            std::fs::File::create(&lib_dir.join(format!("{}_Hdr{}.h", node.lib_name(), i)))
                .unwrap(),
        );

        // for framework in ALL_FRAMEWORKS {
        //     writeln!(hdr_file, "@import {};", framework).unwrap();
        // }
        writeln!(hdr_file, "@import Foundation;").unwrap();
        for child in node.children() {
            writeln!(hdr_file, "@import {};", child.lib_name()).unwrap();
        }

        writeln!(
            hdr_file,
            "@interface {}_Hdr{}_Class : NSObject",
            node.lib_name(),
            i
        )
        .unwrap();
        writeln!(hdr_file, "@end").unwrap();

        let mut m_file = BufWriter::new(
            std::fs::File::create(&lib_dir.join(format!("{}_Src{}.m", node.lib_name(), i)))
                .unwrap(),
        );

        writeln!(
            m_file,
            "#include \"{}/{}_Hdr{}.h\"",
            node.lib_name(),
            node.lib_name(),
            i
        )
        .unwrap();
        writeln!(m_file, "@implementation {}_Hdr{}_Class", node.lib_name(), i).unwrap();
        writeln!(m_file, "@end").unwrap();
    }
}

fn package(v: &Vec<u64>) -> String {
    (1..=v.len())
        .map(|i| format!("pkg_{}", i))
        .intersperse("/".to_string())
        .collect()
}

fn num_nodes_in_ntree(targets_per_level: u64, height: u32) -> u64 {
    (targets_per_level.pow(height + 1) - 1) / (targets_per_level - 1)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    std::fs::remove_dir_all(&args.output).unwrap_or(());
    std::fs::create_dir_all(&args.output)?;

    let targets_per_level = args.targets_per_level as u64;

    // k^{h+1} - 1) / (k - 1 )
    let height = args.height;
    let num_nodes = num_nodes_in_ntree(targets_per_level, height);
    stream::iter(0..num_nodes)
        .for_each_concurrent(64, |i| {
            emit_build_file(
                i,
                args.targets_per_level,
                args.files_per_target,
                args.output.clone(),
                height as u64,
            )
        })
        .await;

    std::fs::copy(
        std::env::home_dir().unwrap().join("GEN_WORKSPACE"),
        args.output.join("WORKSPACE"),
    )
    .unwrap();

    let mut f = std::fs::File::create(args.output.join(".bazelversion")).unwrap();
    writeln!(f, "5.0.0.7").unwrap();

    let mut f = std::fs::File::create(args.output.join("main.m")).unwrap();
    writeln!(f, "int main(int, char*[]){{return  0;}}").unwrap();

    Ok(())
}
