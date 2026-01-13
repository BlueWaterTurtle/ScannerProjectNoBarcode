import re
import pytesseract
from PIL import Image
import os
import logging
from watchdog.observers import Observer
from watchdog.events import FileSystemEventHandler
import time
import uuid
import shutil
import argparse
import sys

def check_dependencies():
    """Ensure necessary external tools are installed."""
    global TESSERACT_CMD
    if getattr(sys, 'frozen', False):  # Running as a PyInstaller bundle
        base_path = sys._MEIPASS  # PyInstaller runtime path
        TESSERACT_CMD = os.path.join(base_path, "Tesseract-OCR", "tesseract.exe")
    else:
        TESSERACT_CMD = shutil.which("tesseract")  # Use system-installed Tesseract if not frozen
    
    if TESSERACT_CMD is None:
        logging.error("Tesseract not found. Please install Tesseract.")
        sys.exit(1)

    os.environ["TESSDATA_PREFIX"] = os.path.dirname(TESSERACT_CMD)
    pytesseract.pytesseract.tesseract_cmd = TESSERACT_CMD  # Bind pytesseract to the executable

class POFileHandler(FileSystemEventHandler):
    def __init__(self, parent_directory, finished_directory, error_directory):
        self.parent_directory = parent_directory
        self.finished_directory = finished_directory
        self.error_directory = error_directory

    def on_created(self, event):
        if not event.is_directory:
            file_path = event.src_path
            logging.info(f"New file detected: {file_path}")
            
            # Wait for the file to finish writing
            while not os.path.exists(file_path) or (
                os.path.isfile(file_path) and os.stat(file_path).st_size == 0
            ):
                logging.info(f"Waiting for file to finish writing: {file_path}")
                time.sleep(1)

            # Process PNG files only
            if file_path.lower().endswith('.png'):
                po_number = extract_po_number_from_image(file_path)
                if po_number:
                    logging.info(f"Extracted PO Number: {po_number}")
                    self.rename_and_move(file_path, po_number)
                else:
                    logging.info("PO Number could not be extracted.")
                    self.handle_no_po_numbers(file_path)
            else:
                logging.warning(f"Unsupported file format detected: {file_path}")

    def rename_and_move(self, file_path, po_number):
        file_extension = os.path.splitext(file_path)[1]  # Get the file extension
        new_file_name = f"{po_number}_{uuid.uuid4().hex[:6]}{file_extension}"  # Create a unique name
        destination_path = os.path.join(self.finished_directory, new_file_name)

        # Ensure finished directory exists
        if not os.path.exists(self.finished_directory):
            os.makedirs(self.finished_directory)
            logging.info(f"Created finished directory: {self.finished_directory}")

        shutil.move(file_path, destination_path)  # Move file to finished directory
        logging.info(f"File renamed to '{new_file_name}' and moved to: {destination_path}")

    def handle_no_po_numbers(self, file_path):
        # Move files with no detectable PO numbers to an error directory
        if not os.path.exists(self.error_directory):
            os.makedirs(self.error_directory)
            logging.info(f"Created error directory: {self.error_directory}")

        error_file_path = os.path.join(self.error_directory, os.path.basename(file_path))
        shutil.move(file_path, error_file_path)
        logging.warning(f"File with no PO number moved to: {error_file_path}")

def extract_po_number_from_image(image_path):
    """
    Extracts the Purchase Order (PO) number from a PNG image using OCR.
    """
    try:
        # Perform OCR on the image
        ocr_result = pytesseract.image_to_string(Image.open(image_path))
        po_number_match = re.search(r'PO\d+', ocr_result)  # Look for "PO" followed by numbers using regex

        if po_number_match:
            logging.info(f"PO Number '{po_number_match.group()}' found in image: {image_path}")
            return po_number_match.group()
        logging.warning(f"No PO Number found in image: {image_path}")
        return None
    except Exception as e:
        logging.error(f"Error processing image {image_path}: {str(e)}")
        return None

if __name__ == "__main__":
    # Parse command-line arguments
    parser = argparse.ArgumentParser(description="Monitor a directory for PO files.")
    parser.add_argument(
        "--root_directory",
        type=str,
        default=os.path.join(os.path.abspath(os.sep), "renamescans"),  # Default to "renamescans" in the root of the drive
        help="Root directory to house 'waves', 'wavesfinished', and 'waveserrors'. Default is root\\renamescans.",
    )
    args = parser.parse_args()

    # Define directories under the parent "renamescans" directory
    parent_directory = args.root_directory
    waves_directory = os.path.join(parent_directory, "waves")
    finished_directory = os.path.join(parent_directory, "wavesfinished")
    error_directory = os.path.join(parent_directory, "waveserrors")

    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s - [%(levelname)s] - %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S"
    )

    check_dependencies()  # Check for Tesseract dependency

    # Ensure all directories exist
    for folder in [parent_directory, waves_directory, finished_directory, error_directory]:
        if not os.path.exists(folder):
            logging.info(f"Directory does not exist. Creating directory: {folder}")
            os.makedirs(folder)

    # Start monitoring the 'waves' directory
    event_handler = POFileHandler(parent_directory, finished_directory, error_directory)
    observer = Observer()
    observer.schedule(event_handler, path=waves_directory, recursive=False)

    try:
        logging.info(f"Monitoring directory: {waves_directory}")
        observer.start()  # Start monitoring
        while True:
            time.sleep(1)  # Keep the script running
    except KeyboardInterrupt:
        logging.info("Stopping directory monitoring.")
        observer.stop()
    observer.join()
